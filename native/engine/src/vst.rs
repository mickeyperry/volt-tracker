//! A VST3 plugin, as a node in the audio graph.
//!
//! This is the join between two worlds: `volt-vsthost` knows how to talk COM to a plugin, and
//! `web-audio-api` owns the graph and the audio callback. An `AudioWorkletProcessor` is the crate's
//! hook for a node whose DSP is written by someone else — which is precisely what a plugin is — so
//! a VST becomes an ordinary node that can be connected anywhere a `GainNode` can, including into
//! `mixer.rs`'s insert slot. **That is the whole architectural bet of the mixer plan, cashed.**
//!
//! Notes reach it the way the spec intends messages to: `node.port().post_message(...)` from the
//! control thread, delivered to `onmessage` on the render thread. They carry an absolute context
//! time, exactly as `pump()`'s lookahead does in the browser, and the processor turns that into a
//! sample offset inside the block it belongs to — so a note lands where it was scheduled, not at
//! the start of whichever block was being rendered when the message arrived. At 256 frames that is
//! the difference between in time and up to 5.8 ms late, which is the whole reason for ASIO.

use std::any::Any;
use std::sync::Arc;

use volt_vsthost::{EditorHandle, HostEvents, Module, Plugin, PluginInfo};
use web_audio_api::context::BaseAudioContext;
use web_audio_api::worklet::{
    AudioParamValues, AudioWorkletGlobalScope, AudioWorkletNode, AudioWorkletNodeOptions,
    AudioWorkletProcessor,
};

/// One scheduled note, control thread → render thread.
#[derive(Debug, Clone, Copy)]
pub struct NoteMsg {
    /// VOLT note number (A4 = 440 at 57). The conversion to MIDI happens in `HostEvents`.
    pub note: f32,
    pub velocity: f32,
    /// Absolute context time, the same clock `trig()` schedules against.
    pub when: f64,
    pub on: bool,
}

/// One parameter change on its way to the render thread — automation's last hop.
///
/// LIVE sends carry `when: 0.0` and apply at the top of the next block, deliberately: parameter
/// automation is block-rate (the browser's mix LFOs are too), and a few milliseconds is
/// indistinguishable from the IPC hop it already took. An OFFLINE render has no "next block that
/// means now" — its whole timeline is booked up front — so a render's lane values arrive with a
/// real `when` and wait in the processor until their block comes round, like notes do.
#[derive(Debug, Clone, Copy)]
pub struct ParamMsg {
    pub id: u32,
    /// Normalised 0..1.
    pub value: f64,
    /// Absolute context time; 0.0 (every live caller) means "next block".
    pub when: f64,
}

/// Where the song is, as an ANCHOR rather than a position.
///
/// A position would be stale by the time it arrived and would have to be re-sent every block. An
/// anchor — "at context time `t0` the song was at beat `beat0`, running at `bpm`" — lets the
/// render thread work out the exact musical position of every block it renders, from the clock's
/// own drift-free timeline, with one message per transport change. `playing:false` stops the
/// plugin's pattern instead of leaving it rolling for ever.
#[derive(Debug, Clone, Copy)]
pub struct TransportMsg {
    pub playing: bool,
    pub t0: f64,
    pub beat0: f64,
    pub bpm: f64,
}

/// The song's tempo, on its way to the plugin's transport.
///
/// Sent on Play and whenever the BPM changes, never per block: it is one store on the far side,
/// and everything tempo-synced inside a plugin (delays, LFOs, gates, arpeggiators) reads it from
/// the transport the host presents every block anyway.
#[derive(Debug, Clone, Copy)]
pub struct TempoMsg {
    pub bpm: f64,
}

/// The panic message: everything off, NOW.
///
/// A note-off per held note is not enough for a panic — the host's book of what is held can be
/// wrong the moment anything has gone wrong, which is exactly when a panic is pressed. So this
/// plays the classic hard version every DAW's panic plays: a note-off for **all 128 MIDI notes**
/// in one block. VST3 has no all-notes-off event and VST2's CC 123 is honoured inconsistently;
/// the sweep is the one form every instrument understands.
#[derive(Debug, Clone, Copy)]
pub struct AllOffMsg;

/// The fresh-start message: everything [`AllOffMsg`] does, plus the plugin's own internal state —
/// ringing voices, delay lines, reverb tails — flushed through each format's convention
/// (`Plugin::flush_state`). Sent on Play so a take starts from silence instead of under the last
/// take's tails, and on panic, where a delay still repeating IS the stuck sound.
#[derive(Debug, Clone, Copy)]
pub struct ResetMsg;

/// "Hand me your patch." Answered on `reply` with the plugin's own state blob, or `None` if it
/// keeps nothing beyond its parameters.
///
/// This is how a preset survives a device reopen. The plugin lives on the render thread and only
/// that thread may speak to it, so the state has to be fetched rather than read — see the handler
/// in `onmessage` for the rule about WHEN it is safe to ask.
pub struct StateMsg {
    pub reply: std::sync::mpsc::Sender<Option<Vec<u8>>>,
}

/// The plugin, on its way to the render thread.
///
/// **Why this is `unsafe impl Send`.** A VST3 plugin is COM: raw pointers, no `Send`. Every DAW
/// creates a plugin on the UI thread and processes it on the audio thread, and the VST3 spec is
/// written around exactly that split — `process` is documented as the audio-thread call and the
/// setup calls as the UI-thread ones. What must never happen is *concurrent* access from both, and
/// that is what this type enforces by construction: the plugin is moved into the processor and the
/// control thread keeps no handle to it afterwards. One move, one owner.
struct PluginBox(Plugin);
// SAFETY: see above — ownership is transferred, never shared.
unsafe impl Send for PluginBox {}

/// Where a plugin goes to be destroyed: back to the thread that made it.
///
/// **A plugin must be destroyed on the thread that created it, exactly as it must be created on
/// the thread that will open its editor.** Measured 2026-08-11 (`examples/editthread.rs`):
/// dropping Sytrus on a thread that did not create it exits 222, Sylenth1 access-violates. VST2
/// has no component/controller split, so its teardown is UI-thread work like the rest of its
/// dispatcher.
///
/// The audio thread therefore never drops a plugin — it hands it back down this channel and the
/// main thread does the destroying. This is the "non-RT reclamation" the engineering bible
/// prescribes (`01_ENGINE_ARCHITECTURE`, edit-to-audio transaction), arrived at from the crash.
pub type Retire = std::sync::mpsc::Sender<Plugin>;

use std::sync::atomic::{AtomicU64, Ordering};
/// Worst single plugin `process` call since it was last read, in microseconds — see the timing
/// in `VstProcessor::process`. Written from the render thread with a relaxed fetch_max (one
/// atomic, no lock, no allocation), read and reset once a second by the desktop's stats monitor.
pub static VST_PEAK_US: AtomicU64 = AtomicU64::new(0);

/// Read and reset the worst plugin block time.
pub fn take_vst_peak_us() -> u64 {
    VST_PEAK_US.swap(0, Ordering::Relaxed)
}

/// What the node needs to build its processor on the render thread.
pub struct VstOptions {
    plugin: PluginBox,
    /// How far ahead notes may be posted. Anything scheduled further out than this is still
    /// honoured; the queue simply holds it.
    channels: usize,
    retire: Retire,
}

/// The render-thread half: holds the plugin, and notes until their block comes round.
pub struct VstProcessor {
    /// `Option` only so `Drop` can take it out and send it home. Never `None` while processing.
    plugin: Option<PluginBox>,
    events: HostEvents,
    /// Notes posted but not yet due. Small and drained in order; a `Vec` beats anything cleverer at
    /// this size and never allocates on the audio thread once it has grown.
    pending: Vec<NoteMsg>,
    /// Timed parameter changes (offline renders — see [`ParamMsg`]), waiting for their block.
    pending_params: Vec<ParamMsg>,
    /// Set by [`AllOffMsg`]; the next block opens with a note-off for every MIDI pitch.
    all_off: bool,
    /// Scratch output buffers for the plugin, allocated once.
    out: Vec<Vec<f32>>,
    channels: usize,
    retire: Retire,
    /// Where the song is, as an anchor — see [`TransportMsg`]. `None` until the engine says.
    transport: Option<TransportMsg>,
}

impl Drop for VstProcessor {
    fn drop(&mut self) {
        // Hand the plugin back rather than dropping it here. If the far end is gone (the app is
        // shutting down) the send fails and the plugin is dropped on this thread after all — which
        // is the same risk the process takes anyway, and `hard_exit` means normal shutdown never
        // reaches this path at all.
        if let Some(p) = self.plugin.take() {
            let _ = self.retire.send(p.0);
        }
    }
}

impl AudioWorkletProcessor for VstProcessor {
    type ProcessorOptions = VstOptions;

    fn constructor(opts: Self::ProcessorOptions) -> Self {
        let channels = opts.channels;
        VstProcessor {
            plugin: Some(opts.plugin),
            events: HostEvents::new(),
            pending: Vec::with_capacity(128),
            pending_params: Vec::new(),
            all_off: false,
            out: (0..channels).map(|_| Vec::with_capacity(256)).collect(),
            channels,
            retire: opts.retire,
            transport: None,
        }
    }

    fn process<'a, 'b>(
        &mut self,
        inputs: &'b [&'a [&'a [f32]]],
        outputs: &'b mut [&'a mut [&'a mut [f32]]],
        _params: AudioParamValues<'b>,
        scope: &'b AudioWorkletGlobalScope,
    ) -> bool {
        let output = &mut outputs[0];
        let frames = output.first().map(|c| c.len()).unwrap_or(0);
        if frames == 0 {
            return true;
        }

        // The block covers [t0, t1). Anything due inside it becomes an event at its own sample
        // offset; anything earlier is late (the message arrived after its time) and goes in at
        // offset 0 rather than being dropped, because a late note is still a note.
        let t0 = scope.current_time;
        let t1 = t0 + frames as f64 / scope.sample_rate as f64;

        // **The musical position of THIS block**, worked out from the anchor the engine sent, on
        // the clock's own timeline. This is what makes a tempo-synced gate or arpeggiator line up
        // with the bar instead of running at the right speed from an arbitrary phase.
        if let Some(tr) = self.transport {
            if let Some(plugin) = self.plugin.as_mut() {
                let beat = if tr.playing { tr.beat0 + (t0 - tr.t0) * tr.bpm / 60.0 } else { tr.beat0 };
                plugin.0.set_transport(tr.playing, beat);
            }
        }

        self.events.clear();
        if self.all_off {
            self.all_off = false;
            // MIDI 0..=127; `note_off` takes VOLT numbering and adds 12 (see HostEvents), so the
            // sweep starts at VOLT note -12 to land exactly on MIDI 0.
            for midi in 0..128 {
                self.events.note_off((midi - 12) as f32, 0);
            }
        }
        self.pending.retain(|m| {
            if m.when >= t1 {
                return true; // not yet
            }
            let offset = (((m.when - t0) * scope.sample_rate as f64) as i32).clamp(0, frames as i32 - 1);
            if m.on {
                self.events.note_on(m.note, m.velocity, offset);
            } else {
                self.events.note_off(m.note, offset);
            }
            false
        });
        // Timed lane values (offline renders) land at the top of the block that covers them —
        // block-rate, exactly the precision the live path has always had.
        if !self.pending_params.is_empty() {
            let plugin = &mut self.plugin;
            self.pending_params.retain(|m| {
                if m.when >= t1 {
                    return true;
                }
                if let Some(p) = plugin.as_mut() {
                    p.0.set_param(m.id, m.value);
                }
                false
            });
        }

        // `None` only between `Drop` taking the plugin and the node going away — go silent rather
        // than reach into an empty slot.
        let Some(plugin) = self.plugin.as_mut() else {
            output.iter_mut().for_each(|c| c.fill(0.0));
            return true;
        };
        // The strip's audio, when this node is an EFFECT in an insert. Instruments are built with
        // no input, so this is empty for them and `process_io` degenerates to the old `process`.
        // The shapes line up exactly — a worklet input IS planar channel slices — so no copy here.
        let input: &[&[f32]] = inputs.first().copied().unwrap_or(&[]);
        // **Time the plugin itself.** The render-capacity meter says a whole block blew its
        // budget; it cannot say WHO. With crackle that only appears on channels carrying a VST
        // and survives every device and buffer size (2026-08-14), the one question worth an
        // atomic on the render thread is whether the plugin's own `process` is the stall or
        // whether it merely raises the floor while something else spikes. Two clock reads per
        // block per plugin is what every DAW's performance meter costs.
        let t_in = std::time::Instant::now();
        let failed = plugin.0.process_io(&self.events, input, &mut self.out, frames).is_err();
        VST_PEAK_US.fetch_max(t_in.elapsed().as_micros() as u64, Ordering::Relaxed);
        if failed {
            // A plugin that fails mid-stream is not allowed to take the session with it: go silent
            // and keep the node alive so the graph shape does not change under the callback.
            output.iter_mut().for_each(|c| c.fill(0.0));
            return true;
        }

        for (i, chan) in output.iter_mut().enumerate() {
            // A mono plugin feeding a stereo node repeats its one channel rather than leaving the
            // right side silent.
            let src = &self.out[i.min(self.channels.saturating_sub(1))];
            chan[..frames].copy_from_slice(&src[..frames]);
        }

        // Always true: an instrument is a source, and it keeps sounding after the last note-on.
        true
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(note) = msg.downcast_ref::<NoteMsg>() {
            self.pending.push(*note);
        } else if let Some(p) = msg.downcast_ref::<ParamMsg>() {
            if p.when > 0.0 {
                // an offline render's lane value — parked until its block, like a note
                self.pending_params.push(*p);
                return;
            }
            // Straight into the plugin's own queue (VST3) or effect (VST2): both formats' halves
            // of `set_param` are audio-thread safe by construction.
            if let Some(plugin) = self.plugin.as_mut() {
                plugin.0.set_param(p.id, p.value);
            }
        } else if let Some(t) = msg.downcast_ref::<TempoMsg>() {
            if let Some(plugin) = self.plugin.as_mut() {
                plugin.0.set_tempo(t.bpm);
            }
        } else if let Some(t) = msg.downcast_ref::<TransportMsg>() {
            self.transport = Some(*t);
            // Apply it now as well as per block, so a plugin asked about the transport between
            // blocks (VST2 plugins ask whenever they feel like it) hears the truth immediately.
            if let Some(plugin) = self.plugin.as_mut() {
                plugin.0.set_transport(t.playing, t.beat0);
            }
        } else if msg.downcast_ref::<AllOffMsg>().is_some() {
            // Notes booked for the future die with the panic too — a panic that lets the next
            // lookahead's notes through was not a panic.
            self.pending.clear();
            self.all_off = true;
        } else if let Some(req) = msg.downcast_ref::<StateMsg>() {
            // The patch, read out of the plugin and sent back to whoever asked.
            //
            // **This allocates, on the render thread, and is deliberate.** It is the only thread
            // that owns the plugin, and a saved patch is worth one block's risk — so the callers
            // are restricted instead: state is read when a device is CLOSING (nothing can glitch,
            // the stream is ending) or while the transport is stopped. Never per block, never
            // while playing.
            let bytes = self.plugin.as_ref().and_then(|p| p.0.state());
            let _ = req.reply.send(bytes);
        } else if msg.downcast_ref::<ResetMsg>().is_some() {
            self.pending.clear();
            self.all_off = true;
            // And the plugin's own memory of the previous take goes with them. Both formats'
            // flush calls are the ones their specs allow from the processing context.
            if let Some(plugin) = self.plugin.as_mut() {
                plugin.0.flush_state();
            }
        }
    }
}

/// A plugin in the graph. Connect it like any other node.
pub struct VstNode {
    node: AudioWorkletNode,
    pub name: String,
    pub latency: u32,
}

impl VstNode {
    /// Load `info` from `module` and put it in `ctx`'s graph.
    ///
    /// `max_block` must be at least the render quantum; it is what the plugin is told to expect and
    /// a block larger than it is an error rather than a glitch.
    ///
    /// Returns the node **and the plugin's editor half**, which the caller must move to the main
    /// thread if it wants a plugin window. It has to come out here, before the plugin disappears
    /// into the processor: once `AudioWorkletNode::new` has taken it there is no way back to it
    /// from any thread but the audio one, and a window cannot be run from there.
    pub fn new(
        ctx: &impl BaseAudioContext,
        module: &Arc<Module>,
        info: &PluginInfo,
        max_block: i32,
        retire: Retire,
    ) -> Result<(VstNode, Option<EditorHandle>), String> {
        let plugin = module.instantiate(info, ctx.sample_rate() as f64, max_block)?;
        Self::from_plugin(ctx, plugin, retire)
    }

    /// Put an already-created plugin into `ctx`'s graph.
    ///
    /// Split out from [`new`](VstNode::new) because **where a plugin is created matters and it is
    /// not here.** A VST2 plugin treats its creating thread as its UI thread and will block
    /// forever in `effEditOpen` if that thread has no message pump — so the desktop app creates
    /// plugins on the main thread and hands them here, to the audio thread, to be wired into the
    /// graph. See the `Send` note on `volt_vsthost::Plugin`.
    pub fn from_plugin(
        ctx: &impl BaseAudioContext,
        mut plugin: Plugin,
        retire: Retire,
    ) -> Result<(VstNode, Option<EditorHandle>), String> {
        let name = plugin.name.clone();
        let latency = plugin.latency;
        let channels = plugin.out_channels.max(1);
        let editor = plugin.take_editor();

        // Spelled out rather than `..default()`: the options carry the plugin itself, which has no
        // Default and should not have one — there is no such thing as a blank VST3 instance.
        //
        // An input on any plugin that declares input channels, not only on effects: connecting
        // nothing to it renders silence (spec), which is exactly what the old input-less node fed
        // the plugin anyway — and it is what lets one node type sit in an instrument slot or a
        // channel's insert without a second code path.
        let options = AudioWorkletNodeOptions {
            number_of_inputs: if plugin.in_channels > 0 { 1 } else { 0 },
            number_of_outputs: 1,
            output_channel_count: vec![channels],
            parameter_data: Default::default(),
            processor_options: VstOptions { plugin: PluginBox(plugin), channels, retire },
            // **A plugin insert is a FIXED-FORMAT device: always exactly stereo in.** The default
            // (`Max` + whatever is connected) let the worklet's input change SHAPE with the
            // graph: native voices connect at every note and drop after their tails, so an
            // insert on a native-instrument channel saw its input flip mono/stereo/silent in
            // time with the notes — and the plugin's own state (filters, oversamplers, meters)
            // ate the discontinuity at every flip. That is why an FX on the Sytrus channel was
            // clean (its source node never disconnects: constant shape) while THE SAME FX on a
            // reese channel distorted on every note, on every device, with the plugin bypassed,
            // while a bit-exact offline render of the same graph measured clean — offline, the
            // voice existed before rendering began, so the shape never changed (2026-08-14, the
            // day of hunting; Mickey's channel-6 screenshot was the clue that closed it).
            // `Explicit` at 2 pins the mix: silence arrives as stereo silence, a mono voice as
            // dual-mono, and the plugin sees one format for its whole life — which is what every
            // DAW promises an insert, and what `ProcessData` already promised it downstream.
            audio_node_options: web_audio_api::node::AudioNodeOptions {
                channel_count: 2,
                channel_count_mode: web_audio_api::node::ChannelCountMode::Explicit,
                channel_interpretation: web_audio_api::node::ChannelInterpretation::Speakers,
            },
        };
        let node = AudioWorkletNode::new::<VstProcessor>(ctx, options);

        Ok((VstNode { node, name, latency }, editor))
    }

    /// Schedule a note. `when` is absolute context time, like everything else in the engine.
    pub fn note_on(&self, note: f32, velocity: f32, when: f64) {
        self.node.port().post_message(NoteMsg { note, velocity, when, on: true });
    }

    pub fn note_off(&self, note: f32, when: f64) {
        self.node.port().post_message(NoteMsg { note, velocity: 0.0, when, on: false });
    }

    /// The panic: drop every queued note and note-off all 128 MIDI pitches next block.
    pub fn all_off(&self) {
        self.node.port().post_message(AllOffMsg);
    }

    /// The fresh start: queued notes dropped, the 128-off sweep, and the plugin's internal state
    /// (voices, delay lines, tails) flushed. See [`ResetMsg`].
    pub fn reset(&self) {
        self.node.port().post_message(ResetMsg);
    }

    /// Automation: one normalised parameter change, applied at the next block.
    /// Read the plugin's patch back, waiting up to `ms` for the render thread to answer.
    ///
    /// `None` covers both "this plugin has no state to give" and "it did not answer in time" —
    /// the caller treats them the same, because both mean there is nothing new to remember and
    /// the last known patch (if any) stands. Never call this per block; see [`StateMsg`].
    pub fn state(&self, ms: u64) -> Option<Vec<u8>> {
        let (reply, rx) = std::sync::mpsc::channel();
        self.node.port().post_message(StateMsg { reply });
        rx.recv_timeout(std::time::Duration::from_millis(ms)).ok().flatten()
    }

    pub fn set_param(&self, id: u32, value: f64) {
        self.node.port().post_message(ParamMsg { id, value, when: 0.0 });
    }

    /// A parameter change at a TIME — for offline renders, whose whole timeline is booked up
    /// front. `when` is absolute context time, like a note's.
    pub fn set_param_at(&self, id: u32, value: f64, when: f64) {
        self.node.port().post_message(ParamMsg { id, value, when });
    }

    /// The song's tempo, so anything sync'd inside the plugin lands on the beat. See [`TempoMsg`].
    pub fn set_tempo(&self, bpm: f64) {
        self.node.port().post_message(TempoMsg { bpm });
    }

    /// Where the song is: at context time `t0` it was at beat `beat0`, running at `bpm`. See
    /// [`TransportMsg`] for why this is an anchor rather than a position.
    pub fn set_transport(&self, playing: bool, t0: f64, beat0: f64, bpm: f64) {
        self.node.port().post_message(TransportMsg { playing, t0, beat0, bpm });
    }

    pub fn node(&self) -> &AudioWorkletNode {
        &self.node
    }
}
