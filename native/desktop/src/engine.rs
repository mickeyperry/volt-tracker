//! The native audio engine, exposed to the webview.
//!
//! This is the step that makes the desktop build something other than a browser in a window.
//! Until now `beta2.html` made every sound through Web Audio inside WebView2 — same engine as the
//! browser, same ~58 ms, and the Rust engine sat next to it playing a hardcoded demo. These
//! commands are the wire between the two: the UI says *which device* and *what to play*, and
//! `volt-engine` — the ported `trig()`, the same one `tests/parity.test.js` grades — plays it
//! through ASIO.
//!
//! **Scope, honestly.** This plays a pattern; it is not yet the transport VOLT drives. There is no
//! automation, and only the ported instruments — but since 2026-08-11 there IS a mixer: every voice
//! and every plugin goes through its track's channel strip and on into the master chain, and the
//! browser's faders drive it over `eng_mix`. What it establishes is the
//! architecture: who owns the clock (Rust), how a pattern crosses (JSON, once, not per row), the
//! playhead coming back the other way (`neng:row`), and that the webview can pick an ASIO device
//! and hear it.
//!
//! The context lives on its own thread and is spoken to over a channel. It has to: an
//! `AudioContext` owns a live audio callback, Tauri commands arrive on a pool thread, and letting
//! several of those touch the graph at once is how you get a crash at the worst moment. One
//! thread owns the audio; everyone else sends it messages.

use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

use volt_engine::{
    clock::Clock,
    mixer::{MixState, Mixer},
    noise_buffer, trig,
    vst::VstNode,
    Ctx, Job, Voice,
};
use web_audio_api::context::{
    AudioContext, AudioContextLatencyCategory, AudioContextOptions, BaseAudioContext,
};
use web_audio_api::media_devices::{enumerate_devices_sync, MediaDeviceInfoKind};
use web_audio_api::node::AudioNode;
use web_audio_api::worklet::{AudioWorkletNode, AudioWorkletNodeOptions, AudioWorkletProcessor};

use tauri::Emitter;

/// The event the engine emits as each row sounds, and `-1` when it stops.
///
/// The webview cannot see the engine's clock — its own `AudioContext` is a different device with a
/// different timeline — so the only honest way for the UI to show where the native transport is, is
/// for the engine to say so. One small event per row, at ~11 per second even at 170 bpm.
pub const ROW_EVENT: &str = "neng:row";

/// Anything the engine could not play, on its way back to the status line.
///
/// The audio thread has no other voice: it runs behind a channel, its `eprintln!`s go to a console
/// nobody has open, and a note it refuses to play is otherwise just a hole in the music. Sent once
/// per distinct message, not once per note — a bad instrument on every row of a looping pattern
/// would otherwise be sixteen notifications a bar, forever.
pub const ERR_EVENT: &str = "neng:err";


#[derive(Serialize)]
pub struct Device {
    pub id: String,
    pub label: String,
    /// "ASIO" or "WASAPI", parsed out of the label the forked crate builds. The UI groups on it,
    /// and it is the difference between 5.8 ms and 12 ms on this machine.
    pub host: String,
}

/// One row of one track, as the webview sends it. A deliberately small subset of VOLT's cell —
/// enough to play, not yet enough to be a song.
#[derive(Deserialize, Clone)]
pub struct Cell {
    pub row: usize,
    /// Which lane this cell is on. VOLT is one voice per track — a new note cuts the one before it
    /// and polyphony is extra lanes — so the engine cannot know what to cut without this.
    #[serde(default)]
    pub track: usize,
    pub instr: String,
    /// MIDI note, or **-1 for a note-OFF**, which is how VOLT ends a held note. An OFF carries no
    /// instrument and nothing else on this struct means anything for it.
    pub note: f32,
    pub vel: f32,
    #[serde(default)]
    pub params: std::collections::HashMap<String, f32>,
    /// `Txx`, in 192nds of a row.
    #[serde(default)]
    pub tx: u8,
    /// Which loaded sample this note plays, for `instr == "sampler"`. Sent as an id rather than
    /// audio because a pattern crosses IPC on every play and a sample crosses once.
    #[serde(default)]
    pub sample: Option<String>,
    /// `Bxx` — play this sample backwards.
    #[serde(default)]
    pub rev: bool,
    /// Which loaded VST3 instrument plays this note, when `instr == "vst"`.
    ///
    /// The slot number IS the browser's instrument index, so instrument 3 holding a plugin is
    /// engine slot 3. That saves inventing a second numbering that has to be kept in step with the
    /// first — the bug where the wrong synth plays is much harder to see than a missing one.
    #[serde(default)]
    pub vst: Option<usize>,
    /// The cell's tracker command, for the ones that shape a VOICE: `G` glide, `C` filter cut,
    /// `U`/`D` pitch slides. Timing/sample commands never appear here — `T` crossed as `tx`, `S`
    /// as the `off` param, `B` as `rev` — and an empty string is a plain note. A reese line is
    /// mostly commands; the engine dropping them was "the reese going berzerk when putting vst
    /// over" (2026-08-13).
    #[serde(default)]
    pub cmd: String,
    /// The command's value column (0..255 as VOLT stores it).
    #[serde(default)]
    pub cmdv: f32,
}

/// One sidechain trigger — a note that makes other channels dip, whether or not it is audible.
///
/// Sent SEPARATELY from the cells because the two lists answer different questions: `cells` is
/// "what should the engine play" (ported instruments and plugins, audible tracks only), and this
/// is "what pumps the mix" — which includes muted tracks (the ghost-kick trick) and instruments
/// the engine cannot play yet. Deriving one from the other would silently lose both cases.
#[derive(Deserialize, Clone, Copy)]
pub struct DuckTrig {
    pub row: usize,
    /// The track that played the note — a channel never ducks itself.
    #[serde(default)]
    pub src: usize,
    /// `Txx`, in 192nds of a row, so an off-grid trigger pumps off-grid.
    #[serde(default)]
    pub tx: u8,
    /// Whether the note's instrument is a kick synth — the legacy source-less trigger.
    #[serde(default)]
    pub kick: bool,
}

#[derive(Deserialize)]
pub struct Pattern {
    pub bpm: f64,
    pub lpb: f64,
    pub rows: usize,
    pub cells: Vec<Cell>,
    /// Sidechain triggers, one per note in the pattern. See [`DuckTrig`].
    #[serde(default)]
    pub ducks: Vec<DuckTrig>,
    /// How many times round. 0 means "until stopped".
    #[serde(default = "one")]
    pub repeats: usize,
    /// Seconds from "now" to the first scheduled row. 0 means the old fixed 0.15.
    ///
    /// This is the two-engines flam fix. The webview computes where ITS transport will sound next —
    /// next row time plus its own ~58 ms of output latency — subtracts this engine's latency, and
    /// sends the difference, so both engines' rows reach the speakers together instead of the
    /// plugin landing a 32nd note early. See nengFollowTransport in beta2.html for the arithmetic.
    #[serde(default)]
    pub start_in: f64,
    /// Which row that first schedule lands on. The webview's pump() has already booked its 160 ms
    /// of lookahead by the time this pattern arrives, so "start at row 0" would be starting in the
    /// browser's past — this starts wherever the browser is about to be instead.
    #[serde(default)]
    pub from_row: usize,
    /// True only when the USER pressed Play — the fresh take that flushes every plugin's tails.
    /// A pattern also arrives here on every mid-play re-send (an FX added, a mode toggled, the
    /// song order turning over), and flushing THEN was audible: every plugin's state wiped in
    /// the middle of playback, on a thread FL's VST2s resent. Absent means what it was: not fresh.
    #[serde(default)]
    pub fresh: bool,
}
fn one() -> usize {
    1
}

#[derive(Serialize)]
pub struct Opened {
    pub label: String,
    pub sample_rate: f32,
    pub latency_ms: f64,
}

/// The render thread's health, FL-style: how much of each callback's budget rendering takes, and
/// how many times it has blown that budget since the device opened. Written by the capacity
/// monitor on its own thread (see `eng_open`), read by `eng_stats` — never touches the audio
/// thread, so it is safe to poll while everything else is busy.
#[derive(Serialize, Clone, Copy, Default)]
pub struct EngStats {
    /// Average render load over the last second, 0..1+ (1 = the whole budget).
    pub avg_load: f64,
    /// Worst single callback over the last second.
    pub peak_load: f64,
    /// Underrun EVENTS since the device opened — consecutive blown callbacks count once, which is
    /// what a "glitch you heard" is. The number FL puts next to the buffer slider.
    pub underruns: u64,
    /// Worst single plugin `process` call in the last second, in microseconds, and the block
    /// budget to compare it against. Together they answer the only question a load spike leaves
    /// open: was it the PLUGIN, or was it everything else?
    pub vst_peak_us: u64,
    pub block_us: u64,
}
static ENG_STATS: Mutex<EngStats> =
    Mutex::new(EngStats { avg_load: 0.0, peak_load: 0.0, underruns: 0, vst_peak_us: 0, block_us: 0 });
/// Which audio thread ENG_STATS belongs to. A dead device's monitor must not keep writing over a
/// live one's numbers — see `AUDIO_GEN`.
static STATS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Bumped once per `eng_open`. Everything a device owns that outlives its own thread — today the
/// capacity monitor — carries its generation and shuts up when it stops being the current one.
static AUDIO_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The previous audio thread's "my context is really closed" handshake.
///
/// **A dropped `AudioContext` KEEPS PLAYING.** The crate's `Drop` deliberately leaks the backend
/// (`// Continue playing the stream if the AudioContext goes out of scope`), which is a
/// convenience for a script and a disaster for a DAW: every reopen — the buffer knob, the
/// self-heal, picking a device — left a ZOMBIE engine rendering the old graph, plugins and held
/// notes included, that no command could ever reach again. That is why the panic could not kill
/// the notes: it silenced the engine we still had a handle to, while an orphan kept playing.
/// So the thread now closes its context explicitly and says so here, and the next open WAITS for
/// that word before it touches the hardware.
static PREV_CLOSED: Mutex<Option<std::sync::mpsc::Receiver<()>>> = Mutex::new(None);

/// Poll the engine's render health. Cheap and lock-tiny; the UI reads it on a timer.
#[tauri::command(async)]
pub fn eng_stats() -> EngStats {
    *ENG_STATS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Is the diagnostic tap recording? Read by the tap node on the render thread (one relaxed
/// atomic), flipped by `eng_tap` from the UI. Armed at open time by `VOLT_TAP`.
pub(crate) static TAP_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Start or stop recording what the engine sends the device — see the palette command.
///
/// The node is always in the graph (it costs a memcpy of a silent block when idle); this only
/// decides whether blocks are shipped to the writer. That way recording can start mid-song
/// without touching the graph while audio is running.
#[tauri::command(async)]
pub fn eng_tap(on: bool) -> bool {
    TAP_ON.store(on, std::sync::atomic::Ordering::Relaxed);
    crate::log!("tap: {}", if on { "RECORDING" } else { "stopped — tap.wav written" });
    on
}

/// One reading of the engine's meters, on its way to the UI.
#[derive(Serialize)]
pub struct MeterData {
    /// Loudest recent sample per strip, in strip order — the channel meters.
    pub peaks: Vec<f32>,
    /// Byte spectrum (0-255 per bin, -100..-30 dB scaling, 1024 bins at fft 2048) of the strip
    /// that was asked about, or of the master. The same shape `getByteFrequencyData` fills, so
    /// the browser's spectrum panel can draw it with no translation.
    pub spec: Vec<u8>,
}

enum Cmd {
    Play(Pattern),
    Stop,
    /// The panic. Everything Stop does, plus an all-notes-off sweep into every plugin — and it is
    /// ANSWERED, so the caller can tell "silence achieved" from "the audio thread is wedged and
    /// nothing was heard". That distinction is the whole reason panic is not just Stop: a panic
    /// is pressed precisely when something has already gone wrong.
    Panic { reply: Sender<()> },
    /// The meters: per-strip peaks, plus one spectrum — a strip's (`Some`) or the master's
    /// (`None`). Answered on `reply`; the UI polls this at drawing rate for whichever channel
    /// its spectrum panel is looking at.
    Meters { chan: Option<usize>, reply: Sender<MeterData> },
    /// Raw mono/stereo PCM for one sampler instrument, keyed by whatever id the UI uses.
    ///
    /// The webview decodes, not Rust. It already has `decodeAudioData` for every format Chromium
    /// understands — wav, mp3, flac, ogg, m4a — and duplicating that in Rust would mean a second
    /// decoder to keep in agreement with the first. So the UI hands over finished f32 samples and
    /// the engine only has to play them.
    Sample { id: String, pcm: Vec<f32>, channels: usize },
    /// An **already-created** plugin, on its way into the graph. Answered on `reply` — the caller
    /// waits, because "the plugin did not load" has to reach the person who just picked it.
    ///
    /// The plugin is made on the main thread and moved here, rather than being loaded here from a
    /// path as it used to be. See `vst_load` for why — in one line: a VST2 plugin treats its
    /// creating thread as its UI thread, and one created on the audio thread hangs forever when
    /// its editor is opened.
    VstAttach {
        slot: usize,
        plugin: Box<volt_vsthost::Plugin>,
        reply: Sender<Result<String, String>>,
    },
    VstUnload { slot: usize },
    /// A plugin EFFECT, on its way into a mixer insert. Same handover contract as `VstAttach` —
    /// created elsewhere, wired here — but keyed by TRACK and POSITION, because an effect belongs
    /// to a place in a channel's chain the way an instrument belongs to a slot. **`track` is -1
    /// for the MASTER bus**, whose chain sits between the ported master chain and the safety
    /// limiter. `pos` orders the chain: `pfx_in → pos 0 → pos 1 → … → pfx_out`.
    FxVstAttach {
        track: i64,
        pos: usize,
        plugin: Box<volt_vsthost::Plugin>,
        reply: Sender<Result<String, String>>,
    },
    FxVstUnload { track: i64, pos: usize },
    /// Automation: one normalised parameter change for an instrument plugin…
    VstParam { slot: usize, id: u32, value: f64 },
    /// …or for a channel/master effect plugin, addressed by its place in the chain.
    FxVstParam { track: i64, pos: usize, id: u32, value: f64 },
    /// A whole mixer snapshot. Wholesale rather than a delta — see `Mixer::apply`.
    Mix(Box<MixState>),
}

/// The automatable parameters of every loaded plugin, captured at load time.
///
/// Keyed `"i{slot}"` for instruments and `"f{track}"` for channel effects (`"f-1"` = master).
/// A registry rather than a question to the plugin, because after loading the plugin lives on
/// the audio thread and its controller must not be called from anywhere else — the list is the
/// one thing the pickers need and it does not change for the life of the instance.
static PARAM_LISTS: Mutex<Option<std::collections::HashMap<String, Vec<ParamInfo>>>> =
    Mutex::new(None);

#[derive(Serialize, Clone)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
}

fn remember_params(key: String, list: &[volt_vsthost::ParamDesc]) {
    let mut reg = PARAM_LISTS.lock().unwrap_or_else(|e| e.into_inner());
    reg.get_or_insert_with(Default::default).insert(
        key,
        list.iter().map(|p| ParamInfo { id: p.id, name: p.name.clone() }).collect(),
    );
}

fn recall_params(key: &str) -> Vec<ParamInfo> {
    PARAM_LISTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(key).cloned())
        .unwrap_or_default()
}

/// Instance tag → the same key `PARAM_LISTS` uses, so a knob touch reported by the host
/// (`volt_vsthost::last_touch`) can be pinned to "instrument 7" or "FX on channel 3".
static PLUGIN_TAGS: Mutex<Option<std::collections::HashMap<u64, String>>> = Mutex::new(None);

fn remember_tag(tag: u64, key: String) {
    let mut reg = PLUGIN_TAGS.lock().unwrap_or_else(|e| e.into_inner());
    reg.get_or_insert_with(Default::default).insert(tag, key);
}

/// The knob the user last touched in any plugin's own window — the fuel for
/// "right-click → automate the knob you just moved". `None` until something has been touched.
#[derive(Serialize)]
pub struct TouchInfo {
    /// `"i{slot}"` or `"f{track}"` (`"f-1"` = the master's effect).
    pub target: String,
    pub id: u32,
    pub name: String,
    pub value: f64,
    pub ms_ago: u64,
}

#[tauri::command(async)]
pub fn vst_touched() -> Option<TouchInfo> {
    let (tag, id, value, ms_ago) = volt_vsthost::last_touch()?;
    let target = PLUGIN_TAGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&tag).cloned())?;
    let name = recall_params(&target)
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.name)
        .unwrap_or_else(|| format!("param {id}"));
    Some(TouchInfo { target, id, name, value, ms_ago })
}

/// Where a channel-FX plugin's editor lives in the plugin thread's slot registry.
///
/// Instrument editors are keyed by instrument slot (0..≈32); channel effects share the registry
/// far above anything an instrument list can reach, sixteen keys per channel so a CHAIN of
/// effects each keeps its own window. Track -1 (the master) maps to the first block. One
/// registry, two key ranges — the alternative was more registries with identical code.
const FX_EDITOR_BASE: usize = 1 << 20;
/// Longest FX-VST chain per channel. Sixteen is the editor-registry stride; the UI stops well
/// before that.
pub const FX_CHAIN_MAX: usize = 16;

fn fx_editor_slot(track: i64, pos: usize) -> usize {
    FX_EDITOR_BASE + ((track + 1).max(0) as usize) * FX_CHAIN_MAX + pos.min(FX_CHAIN_MAX - 1)
}

/// Note-off everything the plugins are holding. `stopAllVoices()`, for things that are not voices.
fn vst_all_off(
    vsts: &std::collections::HashMap<usize, VstNode>,
    held: &mut std::collections::HashMap<usize, (usize, f32)>,
    t: f64,
) {
    for (_, (slot, note)) in held.drain() {
        if let Some(v) = vsts.get(&slot) {
            v.note_off(note, t);
        }
    }
}

/// The name of the plugin now in `slot`, for the status line.
fn node_name(vsts: &std::collections::HashMap<usize, VstNode>, slot: usize) -> String {
    vsts.get(&slot).map(|n| n.name.clone()).unwrap_or_default()
}

/// What the status line says about an insert effect the native mixer cannot build yet.
fn fx_skip_msg(kind: &str) -> String {
    format!(
        "the {kind} effect is not in the native engine yet — a plugin on that channel plays without it"
    )
}

/// Put every plugin on the mixer strip of the track it is played on.
///
/// **A plugin's slot is an instrument number; a mixer strip is a track.** Those are different
/// numbers and conflating them was a real bug: the fader sitting next to a plugin's notes did
/// nothing, because the plugin was wired into the channel whose number happened to match its
/// instrument index. A tracker's mental model is unambiguous — a note on track 2 is heard on
/// channel 2 — so that is what this enforces, working the track out from the notes themselves.
///
/// A plugin played from several tracks lands on the first one it appears on. That is a real
/// limitation (a proper answer needs one strip per source, which the browser does not have either)
/// and it is at least predictable.
///
/// Called on every Play, and cheap: it only touches plugins whose track has actually changed.
fn route_vsts(
    ctx: &AudioContext,
    mixer: &mut Mixer,
    vsts: &std::collections::HashMap<usize, VstNode>,
    cells: &[Cell],
    at: &mut std::collections::HashMap<usize, usize>,
) {
    let mut want: std::collections::HashMap<usize, usize> = Default::default();
    for c in cells {
        if let Some(slot) = c.vst {
            want.entry(slot).or_insert(c.track);
        }
    }
    for (slot, track) in want {
        if at.get(&slot) == Some(&track) {
            continue;
        }
        let Some(node) = vsts.get(&slot) else { continue };
        mixer.ensure(ctx, track + 1);
        // Disconnect everything first: this node has exactly one output edge, into whichever strip
        // it was on, and leaving the old one would sum the plugin into two channels at once.
        node.node().disconnect();
        match mixer.input(track) {
            Some(input) => {
                node.node().connect(input);
            }
            None => {
                node.node().connect(&ctx.destination());
            }
        }
        at.insert(slot, track);
        crate::log!("route: plugin slot {slot} -> mixer channel {track}");
    }
}

/// Re-splice one track's whole FX-VST chain from the map, in `pos` order.
///
/// Called on every attach and unload rather than editing the wiring incrementally: a chain is at
/// most a handful of nodes, the wiring is the only state, and "rebuild from the map" cannot drift
/// from the map the way patch-the-graph code does. Track -1 is the master's chain.
fn resplice_fx_chain(
    ctx: &AudioContext,
    mixer: &mut Mixer,
    fx_vsts: &std::collections::HashMap<(i64, usize), VstNode>,
    track: i64,
) {
    let mut links: Vec<(usize, &VstNode)> = fx_vsts
        .iter()
        .filter(|((t, _), _)| *t == track)
        .map(|((_, p), n)| (*p, n))
        .collect();
    links.sort_by_key(|(p, _)| *p);
    let nodes: Vec<&web_audio_api::worklet::AudioWorkletNode> =
        links.iter().map(|(_, n)| n.node()).collect();
    if track < 0 {
        mixer.set_master_plugin_chain(&nodes);
    } else {
        mixer.ensure(ctx, track as usize + 1);
        mixer.set_plugin_chain(track as usize, &nodes);
    }
}

/// Rebuild the per-track latency sums for plugin delay compensation, and return the widest shift
/// any single note could need (channel chain + master chain + the largest instrument plugin).
///
/// Capped at half a second: a linear-phase monster reporting more does not get to drag the whole
/// scheduling horizon out from under the 10 ms tick — past the cap it plays late and honestly so.
fn recompute_latency(
    fx_vsts: &std::collections::HashMap<(i64, usize), VstNode>,
    vsts: &std::collections::HashMap<usize, VstNode>,
    sr: f64,
    fx_lat: &mut std::collections::HashMap<i64, f64>,
) -> f64 {
    fx_lat.clear();
    for ((t, _), n) in fx_vsts {
        *fx_lat.entry(*t).or_insert(0.0) += n.latency as f64 / sr;
    }
    let master = fx_lat.get(&-1).copied().unwrap_or(0.0);
    let chan_max = fx_lat
        .iter()
        .filter(|(t, _)| **t >= 0)
        .map(|(_, s)| *s)
        .fold(0.0, f64::max);
    let inst_max = vsts
        .values()
        .map(|n| n.latency as f64 / sr)
        .fold(0.0, f64::max);
    (master + chan_max + inst_max).min(0.5)
}

// Creating a plugin lives in `crate::plugins` now, on the thread that is allowed to do it. It was
// here, and briefly ran on the main thread — which fixed VST2 editors and froze the window instead.

/// End every held voice at `t` and move it to the retired list to finish its tail.
fn cut_all(
    live: &mut std::collections::HashMap<usize, Voice>,
    retired: &mut Vec<(f64, Voice)>,
    t: f64,
) {
    for (_, mut v) in live.drain() {
        v.cut(t);
        retired.push((v.quiet_by(t), v));
    }
}

/// Handle to the audio thread. `None` until a device has been opened. The field is private because
/// `Cmd` is an implementation detail of this module.
#[derive(Default)]
pub struct Engine(Mutex<Option<Sender<Cmd>>>);

/// The diagnostic tap — see `VOLT_TAP` in `eng_open`. Interleaves each block and hands it to the
/// writer thread; `try_send` so a slow disk can NEVER stall the render thread (a dropped block is
/// logged by count in the WAV being short, which is itself information).
struct TapProcessor {
    tx: std::sync::mpsc::SyncSender<Vec<f32>>,
}

impl AudioWorkletProcessor for TapProcessor {
    type ProcessorOptions = std::sync::mpsc::SyncSender<Vec<f32>>;
    fn constructor(tx: Self::ProcessorOptions) -> Self {
        TapProcessor { tx }
    }
    fn process<'a, 'b>(
        &mut self,
        inputs: &'b [&'a [&'a [f32]]],
        _outputs: &'b mut [&'a mut [&'a mut [f32]]],
        _params: web_audio_api::worklet::AudioParamValues<'b>,
        _scope: &'b web_audio_api::worklet::AudioWorkletGlobalScope,
    ) -> bool {
        if !TAP_ON.load(std::sync::atomic::Ordering::Relaxed) {
            return true; // idle: the node stays in the graph, costing nothing but this check
        }
        let input = inputs.first().copied().unwrap_or(&[]);
        let frames = input.first().map(|c| c.len()).unwrap_or(128);
        let mut block = vec![0.0f32; frames * 2];
        for (c, chan) in input.iter().take(2).enumerate() {
            for (i, &s) in chan.iter().enumerate() {
                block[i * 2 + c] = s;
            }
        }
        if input.len() == 1 {
            for i in 0..frames {
                block[i * 2 + 1] = block[i * 2]; // mono in: same on both ears, as the device hears
            }
        }
        let _ = self.tx.try_send(block);
        true
    }
}

/// The tap's writer half: buffer up to 60 s, write a 16-bit WAV when the channel closes (device
/// closed or app exit) — and rewrite the file every few seconds so a hard kill still leaves data.
fn tap_writer(rx: std::sync::mpsc::Receiver<Vec<f32>>, sr: u32) {
    let path = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|d| d.join("VOLT").join("tap.wav"));
    let Some(path) = path else { return };
    let mut all: Vec<f32> = Vec::new();
    let cap = (sr as usize) * 2 * 60;
    let mut last_write = std::time::Instant::now();
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(b) => {
                if all.len() < cap {
                    all.extend_from_slice(&b);
                }
                if last_write.elapsed().as_secs() >= 3 {
                    write_wav(&path, &all, sr);
                    last_write = std::time::Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    write_wav(&path, &all, sr);
}

fn write_wav(path: &std::path::Path, interleaved: &[f32], sr: u32) {
    let n = interleaved.len();
    let data_bytes = n * 2;
    let mut out = Vec::with_capacity(44 + data_bytes);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for &s in interleaved {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    let _ = std::fs::write(path, out);
}

/// The enumeration journal — `dev_journal`'s sibling, for the LISTING rather than the open.
///
/// Listing ASIO devices loads every vendor driver, and a driver can access-violate inside
/// `ASIOInit` and kill the process (measured 2026-08-13, twice: WER 0xc0000005 at
/// `volt_desktop!ASIOInit` while the picker was up). Written before an enumeration that will
/// touch ASIO, erased after it survives; a leftover means the last listing died in a driver, so
/// the next ones skip the ASIO host entirely (`VOLT_SKIP_ASIO`, honoured in the vendored fork)
/// until the user explicitly retries from the picker.
fn enum_journal() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|d| d.join("VOLT").join("dev-enum.txt"))
}

#[tauri::command(async)]
pub fn eng_devices(retry_asio: Option<bool>) -> Vec<Device> {
    let jp = enum_journal();
    if retry_asio.unwrap_or(false) {
        if let Some(p) = &jp {
            let _ = std::fs::remove_file(p);
        }
        crate::log!("eng_devices: retrying ASIO enumeration at the user's request");
    }
    let crashed_before = jp.as_ref().map_or(false, |p| p.exists());
    if crashed_before {
        std::env::set_var("VOLT_SKIP_ASIO", "1");
        crate::log!("eng_devices: last listing died inside an ASIO driver — listing WITHOUT ASIO");
    } else {
        std::env::remove_var("VOLT_SKIP_ASIO");
        if let Some(p) = &jp {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            use std::io::Write;
            if let Ok(mut f) = std::fs::File::create(p) {
                let _ = write!(f, "asio-enumeration");
                let _ = f.flush();
            }
        }
    }
    let out: Vec<Device> = enumerate_devices_sync()
        .into_iter()
        .filter(|d| d.kind() == MediaDeviceInfoKind::AudioOutput)
        .map(|d| {
            let label = d.label().to_string();
            // The fork appends "[ASIO]" / "[WASAPI]"; anything else is an older build or a
            // platform without the fork, and "?" is more honest than guessing.
            let host = label
                .rsplit_once('[')
                .and_then(|(_, r)| r.strip_suffix(']'))
                .unwrap_or("?")
                .to_string();
            Device { id: d.device_id().to_string(), label, host }
        })
        .collect();
    // Surviving the walk is what clears the journal — and only when ASIO was actually walked;
    // a WASAPI-only listing proves nothing about the drivers.
    if !crashed_before {
        if let Some(p) = &jp {
            let _ = std::fs::remove_file(p);
        }
    }
    out
}

/// The device-open journal — `plugins.rs`'s vst-loading trick, for audio drivers.
///
/// ASIO4ALL wedged the audio thread INSIDE the driver (2026-08-13: parked forever in wdmaud's
/// GS-wavetable-synth teardown under `midiOutClose`, called from asio4all64.dll) — and it wedged
/// AFTER `Opened` had been sent, so the open looked successful while every later command timed
/// out ("the AUDIO THREAD never answered in 30 s"). A wedged thread cannot be killed; what can be
/// done is REMEMBERING. The device id is written here before an open and erased by the audio
/// thread's first completed command-loop pass — the first moment the thread is PROVEN alive,
/// which a successful-looking open is not. An AUTO open (startup, self-heal) that finds its own
/// device still in the file refuses with the story; choosing the device by hand in the picker
/// always tries again.
fn dev_journal() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|d| d.join("VOLT").join("dev-opening.txt"))
}

/// Open a device and leave an audio thread running on it.
///
/// Opening replaces any previous device: an ASIO driver usually takes exclusive control of the
/// hardware, so holding two contexts open would mean the second one silently failing to make a
/// sound. Dropping the old sender ends the old thread, which drops its context and releases the
/// driver before we ask for the next.
#[tauri::command(async)]
pub fn eng_open(
    app: tauri::AppHandle,
    state: tauri::State<'_, Engine>,
    device_id: String,
    label: Option<String>,
    auto: Option<bool>,
    buf_ms: Option<f64>,
) -> Result<Opened, String> {
    // A device that wedged the audio thread last time is not walked into AGAIN by automation —
    // that is how a bad driver day turns into "VOLT starts broken every time". A human choosing
    // it from the picker overrides; the journal is re-written below either way.
    if auto.unwrap_or(false) {
        if let Some(jp) = dev_journal() {
            if let Ok(prev) = std::fs::read_to_string(&jp) {
                if prev.trim() == device_id {
                    let who = label.as_deref().unwrap_or(&device_id);
                    crate::log!("eng_open: refusing auto-open of {who} — it wedged the audio thread last time");
                    return Err(format!(
                        "{who} wedged the audio engine last time (the driver hung inside Windows' \
                         audio stack). Pick a device — choosing this one again will retry it."
                    ));
                }
            }
        }
    }
    // **Opens are SERIALIZED against each other** — and only against each other, so this cannot
    // recreate the eng_play freeze the engine lock's two-touch rule exists to prevent.
    //
    // Without this, two eng_opens racing (the startup auto-open against a hand-pick, seen in the
    // log 2026-08-14 01:25:05 — one second, two "alive and pumping" lines, one ASIO one WASAPI)
    // each take a different half of the PREV_CLOSED handshake and both walk into a driver. The
    // loser self-heals when its sender is dropped, but for a second there are two engines on two
    // devices — the exact two-owner state everything else here is built to forbid.
    static OPEN_GATE: Mutex<()> = Mutex::new(());
    let _open_gate = OPEN_GATE.lock().unwrap_or_else(|e| e.into_inner());

    // **Take the engine lock twice, briefly, instead of once for the whole open.**
    //
    // It used to be held from here until the driver answered — up to fifteen seconds. Every other
    // command that touches the engine blocks on this mutex, and `eng_play` runs on the main thread,
    // so pressing Play while a device was still opening froze the entire window until the driver
    // replied. Caught in the act 2026-08-11: the app not responding with **zero CPU** across all 44
    // threads, the main thread parked in `NtWaitForAlertByThreadId` — a Rust lock wait, not slow
    // work. Opening is the one slow thing here and it needs no lock at all: the channel it builds
    // is local until it is published at the end.
    {
        let mut slot = state.0.lock().map_err(|_| "engine lock poisoned")?;
        *slot = None;
    }

    let (tx, rx) = channel::<Cmd>();
    let (ready_tx, ready_rx) = channel::<Result<Opened, String>>();
    // This device's generation, and the handshake its thread will use to say it has really let go.
    let gen = AUDIO_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let (closed_tx, closed_rx) = channel::<()>();

    // **Wait for the last engine to actually stop before opening the next one.**
    //
    // The sender was dropped above, so the old thread is on its way out; what matters is that its
    // context is CLOSED (not merely dropped — see PREV_CLOSED) before this one takes the driver.
    // Bounded, because a wedged audio thread must not turn a device change into a frozen window:
    // two seconds is far longer than a healthy close and short enough to stay a UI action. If it
    // does time out we open anyway — the old thread is already the problem, and refusing to open
    // would leave the user with no engine at all.
    let prev = PREV_CLOSED.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(rx_closed) = prev {
        if rx_closed
            .recv_timeout(std::time::Duration::from_secs(2))
            .is_err()
        {
            crate::log!("eng_open: the previous audio thread did not confirm it closed — opening anyway");
        }
    }

    // **No enumeration in here, at all — that is a rule now, learned twice.**
    //
    // First lesson (2026-08-11): the label lookup used to run `enumerate_devices_sync()` after the
    // driver opened, inside the audio thread — walking EVERY ASIO driver while one was held open,
    // blowing the open budget and blaming other software. Moved before the open. Second lesson
    // (2026-08-12): even before the open it was the taskbar light-show Mickey reported — cpal's
    // ASIO enumeration LOADS each driver to ask its name, and every loading driver may flash its
    // tray icon. So the label now simply arrives with the call (the UI remembered it alongside
    // the id), and existence is checked the cheap way: `output_device_available` opens only the
    // host named inside the id and matches by name.
    //
    // The check also closes a quiet trap: with an id that no longer resolves (device unplugged,
    // or a pre-2026-08-12 id from before the format change), the crate's `build_output` would
    // fall back to the DEFAULT output device — opening some other interface than the one the
    // user chose, with a label claiming otherwise. Refusing here keeps the promise honest, and
    // the UI falls back to a real enumeration and a label match.
    // `DEFAULT` — "wherever Windows is playing", the no-drivers-no-fuss option. It maps to the
    // crate's empty sink_id, which resolves through `default_host()` and NEVER picks an ASIO
    // driver (taking exclusive control of hardware nobody named would be the opposite of the
    // point). No availability check either: checking would mean enumeration, and the default
    // device's absence is an honest error from the open itself.
    let is_default = device_id == "DEFAULT";
    if !is_default && !web_audio_api::media_devices::output_device_available(&device_id) {
        return Err(format!(
            "device not found: {}",
            label.as_deref().unwrap_or(&device_id)
        ));
    }
    let label = label.unwrap_or_else(|| "(device)".into());

    // Who we are about to trust, flushed BEFORE the driver gets a chance to wedge us — an
    // unflushed line dies with the process and defeats the whole journal. Erased by the audio
    // thread's first completed loop pass, the proof of life a successful open is not.
    if let Some(jp) = dev_journal() {
        if let Some(dir) = jp.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::File::create(&jp) {
            let _ = write!(f, "{device_id}");
            let _ = f.flush();
        }
    }

    std::thread::spawn(move || {
        // The buffer knob, FL-style. `Interactive` asks the backend for its lowest safe buffer;
        // a chosen length trades latency for headroom — the one control that actually cures
        // crackle on a busy song. A hint only: the driver clamps it to what the hardware can do.
        let hint = match buf_ms {
            Some(ms) if ms > 0.0 => AudioContextLatencyCategory::Custom(ms / 1000.0),
            _ => AudioContextLatencyCategory::Interactive,
        };
        let ctx = AudioContext::new(AudioContextOptions {
            latency_hint: hint,
            sink_id: if is_default { String::new() } else { device_id.clone() },
            ..AudioContextOptions::default()
        });

        // Breadcrumbs, one per startup stage — because the audio thread has now wedged between
        // "open reported success" and "first loop pass" on two different drivers, and the only
        // honest way to find WHERE is to make the thread say how far it got (2026-08-13).
        crate::log!("audio thread: context open");
        let sr = ctx.sample_rate();
        let latency_ms = (ctx.base_latency() + ctx.output_latency()) * 1000.0;
        // Published so `vst_load` can set a plugin up for this device without touching the context
        // from another thread.
        *SAMPLE_RATE.lock().unwrap_or_else(|e| e.into_inner()) = sr as f64;
        // The buffer ASKED for and the latency actually GOT, in one line — because "is the buffer
        // setting even reaching the driver?" was unanswerable from this log while chasing crackle
        // on 2026-08-14, and a latency hint is only ever a hint (WASAPI shared mode in particular
        // does as it pleases).
        // How many channels the DEVICE actually gave us, and what the destination will accept.
        // "Only the left side, silent when panned right" is what a ONE-channel destination does
        // to a panned stereo bus under discrete interpretation, and nothing in the mixer can be
        // blamed for it — so the number gets said out loud at every open (2026-08-15).
        {
            let d = ctx.destination();
            crate::log!(
                "audio thread: destination channels = {} (max {})",
                d.channel_count(),
                d.max_channel_count()
            );
        }
        crate::log!(
            "audio thread: latency read, {sr} Hz — asked for {} , got {:.1} ms",
            match buf_ms {
                Some(ms) if ms > 0.0 => format!("{ms:.1} ms"),
                _ => "the driver minimum".to_string(),
            },
            latency_ms
        );

        if ready_tx
            .send(Ok(Opened { label, sample_rate: sr, latency_ms }))
            .is_err()
        {
            // caller gave up; nothing to play to — but close, or this context plays on for ever
            ctx.close_sync();
            let _ = closed_tx.send(());
            return;
        }

        // The health monitor. The crate keeps per-callback stats already; this asks it to fold
        // them once a second and parks the result in ENG_STATS for `eng_stats` to read. The
        // capacity's own thread dies with the context, so a reopened device starts a fresh count —
        // and the reset below keeps a dead device from showing the last one's sins.
        *ENG_STATS.lock().unwrap_or_else(|e| e.into_inner()) = EngStats::default();
        STATS_GEN.store(gen, std::sync::atomic::Ordering::SeqCst);
        {
            let pstats = ctx.playback_stats();
            let cap = ctx.render_capacity();
            cap.set_onupdate(move |e| {
                // Only the CURRENT device's monitor speaks. A monitor outlives its context by a
                // beat, and two of them writing one static made the counters fight — each seeing
                // the other's number as "before" and logging a fresh underrun a dozen times a
                // second. The log spam was the tell that two engines were alive at once.
                if STATS_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen {
                    return;
                }
                let ur = pstats.underrun_events();
                // The render quantum is fixed at 128 frames; that is the budget one plugin call
                // has to fit inside, alongside everything else in the graph.
                let block_us = (128.0 / sr as f64 * 1_000_000.0) as u64;
                let vst_peak_us = volt_engine::vst::take_vst_peak_us();
                let mut s = ENG_STATS.lock().unwrap_or_else(|p| p.into_inner());
                let before = s.underruns;
                s.avg_load = e.average_load;
                s.peak_load = e.peak_load;
                s.underruns = ur;
                s.vst_peak_us = vst_peak_us;
                s.block_us = block_us;
                // A plugin that eats most of a block on its own IS the stall, and saying so in
                // the log is the difference between a diagnosis and another theory.
                // Reported whenever a plugin ran at all, not only past some threshold: the first
                // version only spoke above 50% of the budget and therefore said NOTHING through
                // the entire 2026-08-14 hunt, which left "are the plugins expensive?" unanswered
                // while everything else was being eliminated. A number every second beats a
                // silence that means two different things.
                if vst_peak_us > 0 {
                    crate::log!(
                        "audio: worst plugin block {vst_peak_us} us of {block_us} us ({:.0}% of one \
                         block) · render load avg {:.0}% peak {:.0}%",
                        vst_peak_us as f64 / block_us as f64 * 100.0,
                        e.average_load * 100.0,
                        e.peak_load * 100.0
                    );
                }
                if ur > before {
                    // once per new event, in the log where freezes get diagnosed — a counter that
                    // only lives in a tooltip is a counter nobody can correlate with a timestamp
                    crate::log!(
                        "audio: underrun #{ur} (load avg {:.0}% peak {:.0}%)",
                        e.average_load * 100.0,
                        e.peak_load * 100.0
                    );
                }
            });
            cap.start(web_audio_api::AudioRenderCapacityOptions { update_interval: 1.0 });
        }

        // Built once per device, not per note: the noise buffer is two seconds of samples and every
        // hat, snare and clap shares it, exactly as `buildGraph()` does in the browser.
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        crate::log!("audio thread: noise built");

        // The browser's `pump()` ticks at 30 ms against 160 ms of lookahead. Scheduling is
        // unchanged by ticking faster — `Clock::due` books whatever falls inside the horizon and
        // nothing twice — but the playhead below can only be as fine as the tick, and 30 ms of
        // stutter in a moving cursor is visible. 10 ms costs a hundred wake-ups a second doing
        // almost nothing, and leaves sixteen ticks of margin instead of five.
        const TICK: f64 = 0.010;
        const LOOKAHEAD: f64 = 0.160;
        let mut clock = Clock::new(120.0, 4.0, 16);
        let mut cells: Vec<Cell> = Vec::new();
        let mut ducks: Vec<DuckTrig> = Vec::new();
        // One voice per track, exactly as `chans[tr].voice` works in the browser: a new note cuts
        // whatever that lane was holding, a note-OFF releases it, and chords are extra lanes.
        // Without this every note simply piled up — sixty-four overlapping copies of a bar-long
        // loop, which is not a subtle difference.
        let mut live: std::collections::HashMap<usize, Voice> = Default::default();
        // Voices that have been cut or released and are finishing their tails. Kept until they are
        // certainly silent, then dropped — which is what frees their nodes.
        let mut retired: Vec<(f64, Voice)> = Vec::new();
        let mut left = 0usize; // rows still to schedule; usize::MAX for "until stopped"
        // Rows booked but not yet audible, and the last row reported to the UI (-1 = none).
        // Rows are scheduled up to 160 ms early, so announcing one when it is *booked* would put
        // the cursor most of a row ahead of the sound. They wait here until their time arrives.
        let mut pending: std::collections::VecDeque<(usize, f64)> = Default::default();
        let mut last_row: i64 = -1;
        // When the last booked row stops sounding — so a run that ends of its own accord clears the
        // cursor after its final row, not during it.
        let mut tail_t = 0.0f64;
        // Problems already reported, so the same one is not sent every row.
        let mut said: std::collections::HashSet<String> = Default::default();
        // Samples live here for the life of the device, so a note costs a lookup rather than a
        // copy. Keyed by the id the UI chose (VOLT uses the sample's SHA-256, which means the
        // same audio used by four instruments is sent and stored exactly once).
        let mut samples: std::collections::HashMap<String, web_audio_api::AudioBuffer> =
            std::collections::HashMap::new();
        // Loaded plugins, by instrument slot. A VstNode owns its plugin and, through it, the DLL
        // it came from — so replacing a slot drops the old plugin, and dropping this map at the
        // end of the thread releases every one of them before the context goes.
        let mut vsts: std::collections::HashMap<usize, VstNode> = Default::default();
        // Plugin EFFECTS, by (mixer track, position in that channel's chain); track -1 = the
        // master bus. Same ownership rules; wired into an insert rather than played by notes.
        // The whole chain for a track is re-spliced from this map whenever any link changes.
        let mut fx_vsts: std::collections::HashMap<(i64, usize), VstNode> = Default::default();
        // ---- plugin delay compensation -----------------------------------------------------
        // A limiter with lookahead (L2: thousands of samples) delays everything through it, and
        // the ear catches a drum bus 40 ms late instantly ("u hear clearly that there is
        // latency"). Every plugin already reports its latency (VstNode::latency); nothing here is
        // live input, everything is scheduled ahead — so instead of delaying every OTHER channel
        // like a DAW with a mic must, notes are simply scheduled EARLY by what their path
        // reports: the channel chain's total plus the master chain's (plus the instrument
        // plugin's own, for a vst note). The wet audio then lands ON the row. Rebuilt from
        // `fx_vsts`/`vsts` whenever a plugin comes or goes.
        let mut fx_lat: std::collections::HashMap<i64, f64> = Default::default(); // seconds per track, -1 = master
        let mut max_shift = 0.0f64; // widest early-schedule any cell might need — extends the clock horizon
        // Plugin delay compensation, off for the session when `VOLT_NO_PDC` is set. A diagnostic
        // switch, not a setting: it makes plugin channels play LATE by their own latency, which
        // is wrong on purpose so that "is the shift what distorts?" can be answered by ear.
        let no_pdc = std::env::var_os("VOLT_NO_PDC").is_some();
        if no_pdc {
            crate::log!("audio thread: PLUGIN DELAY COMPENSATION IS OFF (VOLT_NO_PDC) — diagnostic run");
        }
        // What each lane is currently holding on a plugin: (slot, note). A VST manages its own
        // polyphony, so VOLT's one-voice-per-track rule has to be enforced by the host sending the
        // note-off — otherwise a track's notes pile up inside the synth and never stop.
        let mut vst_held: std::collections::HashMap<usize, (usize, f32)> = Default::default();
        // Which mixer channel each plugin is currently wired into, so a re-route only happens when
        // the track actually changes. See `route_vsts`.
        let mut vst_chan: std::collections::HashMap<usize, usize> = Default::default();
        // Where a finished plugin goes to be destroyed: the plugin thread, which created it. This
        // thread must never drop one — see `volt_engine::vst::Retire`.
        let retire_tx = plugins::retire_sender();

        // **The mixer.** Everything audible goes through it: voices into their track's strip,
        // plugins into theirs, both send buses and the master chain behind them. Built once per
        // device, because it owns nodes and nodes belong to one context.
        let mut mixer = Mixer::new(&ctx);
        mixer.master.connect(&ctx.destination());
        // **`VOLT_TAP=1` records exactly what the device is being sent, to %LOCALAPPDATA%\VOLT\
        // tap.wav.** The 2026-08-14/15 distortion hunt exhausted every indirect measurement —
        // parity nulls, render load, plugin block times, offline strip renders, master peaks —
        // and every one came back clean while the speakers disagreed. A tap ends the argument:
        // a worklet rides the same post-limiter signal spk_out feeds the hardware, ships every
        // block to a writer thread, and the WAV shows the artifact itself — clipped flat tops
        // mean level, zero-gaps mean dropped blocks, doubles mean two sources, garbage means a
        // memory bug. Diagnostic only: nothing is wired unless the env var is set.
        {
            if std::env::var_os("VOLT_TAP").is_some() {
                TAP_ON.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let (tap_tx, tap_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(1024);
            let tap = AudioWorkletNode::new::<TapProcessor>(
                &ctx,
                AudioWorkletNodeOptions {
                    number_of_inputs: 1,
                    number_of_outputs: 0,
                    output_channel_count: vec![],
                    parameter_data: Default::default(),
                    processor_options: tap_tx,
                    audio_node_options: web_audio_api::node::AudioNodeOptions {
                        channel_count: 2,
                        channel_count_mode: web_audio_api::node::ChannelCountMode::Explicit,
                        channel_interpretation: web_audio_api::node::ChannelInterpretation::Speakers,
                    },
                },
            );
            mixer.master.connect(&tap);
            std::mem::forget(tap); // diagnostic node, lives as long as the context does
            let tap_sr = sr as u32;
            std::thread::spawn(move || tap_writer(tap_rx, tap_sr));
            crate::log!(
                "audio thread: tap ready ({}) — palette: \"Record what the engine is playing\"",
                if TAP_ON.load(std::sync::atomic::Ordering::Relaxed) { "RECORDING" } else { "idle" }
            );
        }
        crate::log!("audio thread: mixer built");
        // Scratch for the meter reads, so polling the meters allocates once, not per poll.
        let mut meter_scratch: Vec<f32> = Vec::new();
        let mut last_peak_log = 0.0f64; // throttles the master-level report to once a second
        // Whatever the UI last told us the mixer looked like. A device opened mid-session must come
        // up with the song's faders already set, not at defaults — otherwise switching to ASIO
        // silently resets the mix.
        if let Some(state) = MIX_STATE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            for kind in mixer.apply(&ctx, &state) {
                let msg = fx_skip_msg(&kind);
                let _ = app.emit(ERR_EVENT, msg);
            }
        }
        crate::log!("audio thread: mix applied");

        // Reaching the loop is the audio thread's proof of life — the open, the mixer build and
        // the driver's own post-open games are all behind us — so the device journal comes off
        // here and NOT when `Opened` was sent: ASIO4ALL wedged between the two (2026-08-13).
        if let Some(jp) = dev_journal() {
            let _ = std::fs::remove_file(jp);
        }
        crate::log!("audio thread: alive and pumping ({device_id})");

        loop {
            // Drain commands without blocking while there is anything to do, so playback keeps its
            // tick; `recv_timeout` when idle avoids a spin loop doing nothing. A dropped sender
            // means the app is closing or another device is being opened, and this thread must end
            // so its context is dropped and the driver released — an ASIO driver held by a thread
            // nobody can reach is the one failure that needs the Task Manager to fix.
            // `last_row >= 0` counts as busy: a cursor left on screen still has to be retired, and
            // that happens a row after the last one sounded.
            let busy = clock.playing() || !pending.is_empty() || last_row >= 0;
            let cmd = if busy {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            } else {
                match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(c) => Some(c),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            };
            match cmd {
                Some(Cmd::Play(p)) => {
                    // **A song-order turnover is a HANDOVER, not a new take.**
                    //
                    // The next pattern is sent while the current one is still sounding — the
                    // browser posts it as it schedules the boundary row, which is up to a
                    // lookahead early. Treating that like a Play meant cutting every voice and
                    // note-offing every plugin THE MOMENT IT ARRIVED, so the last ~160 ms of
                    // every pattern was chopped and the next one seemed to arrive early, with a
                    // stumble each time the order looped (2026-08-15: "it goes to the next
                    // pattern too early and too slow getting back to the 1st"). The new pattern
                    // is still armed here so its rows schedule ahead of time; what stops is
                    // killing the audio that is legitimately still ringing. `fresh` (the user
                    // pressing Play) keeps the hard reset it asks for.
                    let handover = clock.playing() && !p.fresh;
                    clock = Clock::new(p.bpm.max(1.0), p.lpb.max(1.0), p.rows.max(1));
                    cells = p.cells;
                    ducks = p.ducks;
                    // The notes say which track each plugin belongs on, so this is the moment its
                    // mixer channel can be known. See `route_vsts`.
                    route_vsts(&ctx, &mut mixer, &vsts, &cells, &mut vst_chan);
                    left = if p.repeats == 0 { usize::MAX } else { p.rows.max(1) * p.repeats };
                    if !handover {
                        cut_all(&mut live, &mut retired, ctx.current_time());
                        vst_all_off(&vsts, &mut vst_held, ctx.current_time());
                    }
                    // A USER Play starts from SILENCE: queued notes from the last take dropped,
                    // and every plugin's internal state — ringing voices, delay repeats, reverb
                    // tails — flushed ("when i hit play i need the vsts to reset so it will
                    // start fresh and not carry shit from delays and tail"). Effects included:
                    // theirs are exactly the delay tails in question. GATED on `fresh`: patterns
                    // also arrive on mid-play re-sends and song-order turnover, where a full
                    // flush is an audible wipe, not a fresh start.
                    if p.fresh {
                        for v in vsts.values() {
                            v.reset();
                        }
                        for v in fx_vsts.values() {
                            v.reset();
                        }
                    } else if !handover {
                        for v in vsts.values() {
                            v.all_off();
                        }
                    }
                    // The cursor's booked rows belong to the pattern still sounding — dropping
                    // them at a handover made the playhead jump before the music did.
                    if !handover {
                        pending.clear();
                    }
                    said.clear();   // a new take reports its problems again
                    // Clamped low, not high: an alignment early enough to be unschedulable gets the
                    // minimum rather than an error, because a slightly-flammed take beats silence.
                    let start_in = if p.start_in > 0.0 { p.start_in.max(0.03) } else { 0.15 };
                    clock.start(ctx.current_time() + start_in, p.from_row);
                }
                Some(Cmd::Stop) => {
                    clock.stop();
                    // `stopAllVoices()`, beta2.html:2012 — cut, not drop. Dropping a voice does not
                    // silence it: a source with a tail keeps rendering whether or not anyone holds
                    // a handle, so a held `sub` would have gone on humming after Stop.
                    cut_all(&mut live, &mut retired, ctx.current_time());
                    // Stop has to reach inside the plugins too. A VST holding a note does not stop
                    // because the transport did — it has no idea there is a transport — so a
                    // forgotten note-off here is a synth droning until the app is closed.
                    vst_all_off(&vsts, &mut vst_held, ctx.current_time());
                    // And the notes BOOKED for the future die here too. They sit in each node's
                    // queue with timestamps past "now" — wider than ever with PDC fetching ahead —
                    // and a note-on that fires after its note-off was sent is a note that sticks
                    // until Ctrl+R ("i get the vsts stuck at times"). Panic always did this;
                    // Stop is what people actually press.
                    for v in vsts.values() {
                        v.all_off();
                    }
                    pending.clear();
                    tail_t = 0.0; // stopped by hand: clear the cursor now, not after a row
                }
                Some(Cmd::Panic { reply }) => {
                    // Stop, and then some. The book of held notes (`vst_held`) can be WRONG by the
                    // time anyone reaches for panic — that is what panic is for — so on top of the
                    // bookkept note-offs, every plugin gets the all-notes-off sweep: 128 note-offs
                    // next block, queued notes dropped (`VstNode::all_off`).
                    clock.stop();
                    cut_all(&mut live, &mut retired, ctx.current_time());
                    vst_all_off(&vsts, &mut vst_held, ctx.current_time());
                    // Panic goes one further than the sweep now: full reset, effects included —
                    // a delay plugin still repeating IS the stuck sound the panic is for.
                    for v in vsts.values() {
                        v.reset();
                    }
                    for v in fx_vsts.values() {
                        v.reset();
                    }
                    // Duckers back to unity too, as the browser's panic does — a mix frozen at
                    // the bottom of a pump is quiet in the wrong way.
                    mixer.duck_reset(ctx.current_time());
                    pending.clear();
                    tail_t = 0.0;
                    // The answer is the point: it proves this thread is alive and the silence was
                    // actually ordered, which is what lets the caller escalate when it was not.
                    let _ = reply.send(());
                }
                Some(Cmd::Meters { chan, reply }) => {
                    let peaks = mixer.peaks(&mut meter_scratch);
                    let mut spec = Vec::new();
                    mixer.spectrum(chan, &mut spec);
                    let _ = reply.send(MeterData { peaks, spec });
                }
                Some(Cmd::Sample { id, pcm, channels }) => {
                    // The webview sends interleaved frames at the *context's* rate — it resamples
                    // on its side with an OfflineAudioContext, because doing it here would mean
                    // writing a resampler whose quality nobody has measured.
                    let ch = channels.max(1);
                    let frames = pcm.len() / ch;
                    if frames > 0 {
                        let mut buf = ctx.create_buffer(ch, frames, sr);
                        for c0 in 0..ch {
                            let chan: Vec<f32> =
                                (0..frames).map(|i| pcm[i * ch + c0]).collect();
                            buf.copy_to_channel(&chan, c0);
                        }
                        samples.insert(id, buf);
                    }
                }
                Some(Cmd::VstAttach { slot, plugin, reply }) => {
                    // Logged on ARRIVAL, before anything is done with it. When a load times out the
                    // question is always "did the audio thread ever see it?" — if this line is
                    // missing the thread never got the command (blocked or dead); if it is present
                    // and no "is playing" follows, building the graph node is what wedged.
                    crate::log!("attach: slot {slot} arrived at the audio thread");
                    // The editor half was already taken on the main thread, where it belongs; what
                    // arrives here is purely the audio side.
                    let r = VstNode::from_plugin(&ctx, *plugin, retire_tx.clone())
                        .map(|(node, _no_editor_here)| {
                            // Provisionally onto track 0's strip, so a plugin is never orphaned
                            // outside the mixer. `Cmd::Play` moves it to the strip of the track it
                            // is actually played on — see `route_vsts`, and the bug that made
                            // necessary: a plugin used to be wired to `mixer.input(slot)`, and a
                            // SLOT IS AN INSTRUMENT NUMBER, NOT A TRACK. Instrument 5's plugin
                            // went into track 5's channel whatever track its notes were on, so
                            // the fader next to the notes did nothing. Mickey: *"i have no
                            // control on volume and stuff at the mixer"*.
                            mixer.ensure(&ctx, 1);
                            match mixer.input(0) {
                                Some(input) => node.node().connect(input),
                                None => node.node().connect(&ctx.destination()),
                            };
                            // Anything the old plugin in this slot was holding is gone with it,
                            // and so is whatever channel it was routed to.
                            vst_held.retain(|_, (s, _)| *s != slot);
                            vst_chan.remove(&slot);
                            vsts.insert(slot, node);
                            node_name(&vsts, slot)
                        });
                    crate::log!("attach: slot {slot} {}", match &r {
                        Ok(n) => format!("in the graph as {n}"),
                        Err(e) => format!("REFUSED: {e}"),
                    });
                    // An instrument plugin's own latency joins the compensation sums too — a
                    // synth is usually 0, but nothing says it has to be.
                    max_shift = recompute_latency(&fx_vsts, &vsts, sr as f64, &mut fx_lat);
                    if no_pdc { max_shift = 0.0; }
                    let _ = reply.send(r);
                }
                Some(Cmd::VstUnload { slot }) => {
                    vsts.remove(&slot);
                    vst_held.retain(|_, (s, _)| *s != slot);
                    vst_chan.remove(&slot);
                    max_shift = recompute_latency(&fx_vsts, &vsts, sr as f64, &mut fx_lat);
                    if no_pdc { max_shift = 0.0; }
                }
                Some(Cmd::FxVstAttach { track, pos, plugin, reply }) => {
                    crate::log!("fx attach: track {track} pos {pos} arrived at the audio thread");
                    let r = VstNode::from_plugin(&ctx, *plugin, retire_tx.clone()).map(
                        |(node, _editor_taken_on_plugin_thread)| {
                            // The old effect's wiring goes before the old effect does, so no
                            // block renders through a node that is about to be dropped.
                            if let Some(old) = fx_vsts.remove(&(track, pos)) {
                                old.node().disconnect();
                            }
                            let name = node.name.clone();
                            let lat = node.latency;
                            fx_vsts.insert((track, pos), node);
                            resplice_fx_chain(&ctx, &mut mixer, &fx_vsts, track);
                            max_shift = recompute_latency(&fx_vsts, &vsts, sr as f64, &mut fx_lat);
                    if no_pdc { max_shift = 0.0; }
                            (name, lat)
                        },
                    );
                    crate::log!("fx attach: track {track} pos {pos} {}", match &r {
                        Ok((n, lat)) => format!(
                            "in the chain as {n} ({lat} samples latency{})",
                            if *lat > 0 { ", compensated" } else { "" }
                        ),
                        Err(e) => format!("REFUSED: {e}"),
                    });
                    let _ = reply.send(r.map(|(n, _)| n));
                }
                Some(Cmd::FxVstUnload { track, pos }) => {
                    if let Some(old) = fx_vsts.remove(&(track, pos)) {
                        old.node().disconnect();
                    }
                    // Re-splice what is left — an empty chain is the straight wire, so the
                    // channel never goes silent with a plugin gone.
                    resplice_fx_chain(&ctx, &mut mixer, &fx_vsts, track);
                    max_shift = recompute_latency(&fx_vsts, &vsts, sr as f64, &mut fx_lat);
                    if no_pdc { max_shift = 0.0; }
                }
                Some(Cmd::VstParam { slot, id, value }) => {
                    if let Some(v) = vsts.get(&slot) {
                        v.set_param(id, value);
                    }
                }
                Some(Cmd::FxVstParam { track, pos, id, value }) => {
                    if let Some(v) = fx_vsts.get(&(track, pos)) {
                        v.set_param(id, value);
                    }
                }
                Some(Cmd::Mix(state)) => {
                    // An unported insert effect is skipped, and the skip is SAID — through the
                    // same deduplicated channel a bad note uses. Silent divergence between what
                    // the panel shows and what a plugin channel sounds like is the old bug
                    // ("if ill add effects it wont sound different") wearing a smaller hat.
                    for kind in mixer.apply(&ctx, &state) {
                        let msg = fx_skip_msg(&kind);
                        if said.insert(msg.clone()) {
                            let _ = app.emit(ERR_EVENT, msg);
                        }
                    }
                }
                None => {}
            }

            if clock.playing() {
                // The horizon grows by the widest compensation in play: a note that must be
                // scheduled 90 ms early has to be FETCHED at least 90 ms earlier than the plain
                // lookahead would fetch it, or the shift below clamps and the note lands late.
                let now = ctx.current_time();
                for d in clock.due(now, LOOKAHEAD + max_shift) {
                    if left == 0 {
                        clock.stop();
                        break;
                    }
                    if left != usize::MAX {
                        left -= 1;
                    }
                    pending.push_back((d.row, d.t));
                    tail_t = d.t + clock.row_dur();
                    // Sidechain triggers first, so a note and the dip it causes on other channels
                    // are scheduled against the same instant. `Mixer::duck` decides who dips.
                    for dk in ducks.iter().filter(|k| k.row == d.row) {
                        mixer.duck(d.t + clock.t_offset(dk.tx), dk.src, dk.kick);
                    }
                    for cell in cells.iter().filter(|c| c.row == d.row) {
                        // `Txx` — this cell's events fire x/192 of a row late, which is how VOLT
                        // gets triplets off an integer row grid.
                        let tm = d.t + clock.t_offset(cell.tx);
                        // ---- plugin delay compensation ------------------------------------
                        // Everything this note will pass through that REPORTS latency: the
                        // channel chain's plugins, then the master's. Scheduling it early by
                        // exactly that puts the wet audio ON the row instead of behind it.
                        // Clamped at "now": a first row too close to Play cannot be reached
                        // early, so it lands late once rather than never.
                        // `VOLT_NO_PDC=1` turns compensation OFF for the whole session — Mickey's
                        // own hypothesis on 2026-08-14 ("can we test maybe without delay
                        // compensation?") and the cheapest way to settle it: with it off, notes
                        // are scheduled at musical time and land late by the chain's latency
                        // instead of early. If the distortion goes with it, the shift is the
                        // cause; if it stays, PDC is eliminated as a suspect for good.
                        let base_shift = if no_pdc {
                            0.0
                        } else {
                            fx_lat.get(&-1).copied().unwrap_or(0.0)
                                + fx_lat.get(&(cell.track as i64)).copied().unwrap_or(0.0)
                        };
                        let inst_lat = |slot: &usize| {
                            vsts.get(slot).map_or(0.0, |v| v.latency as f64 / sr as f64)
                        };
                        // A note-OFF releases what the lane is holding and does nothing else:
                        // beta2.html:2381, `if(cell.n===OFF){if(ch.voice){ch.voice.release(tc)}}`.
                        // It shifts by the same path as the note it ends, or held notes would
                        // stretch by the chain's latency.
                        if cell.note < 0.0 {
                            let held_lat =
                                vst_held.get(&cell.track).map_or(0.0, |(s, _)| inst_lat(s));
                            let tt = (tm - base_shift - held_lat).max(now + 0.002);
                            if let Some(mut v) = live.remove(&cell.track) {
                                v.release(tt);
                                retired.push((v.quiet_by(tt), v));
                            }
                            // An OFF carries no instrument, so a lane holding a plugin note has to
                            // be found by lane, not by what the cell says.
                            if let Some((slot, note)) = vst_held.remove(&cell.track) {
                                if let Some(v) = vsts.get(&slot) {
                                    v.note_off(note, tt);
                                }
                            }
                            continue;
                        }

                        // ---- a plugin note ------------------------------------------------
                        // Handled before `trig()` because it is not a voice at all: the plugin is a
                        // node that is always there, and a note is a message to it.
                        if cell.instr == "vst" {
                            let slot = cell.vst.unwrap_or(usize::MAX);
                            let Some(v) = vsts.get(&slot) else {
                                let msg = format!("vst: nothing loaded in slot {slot}");
                                if said.insert(msg.clone()) {
                                    let _ = app.emit(ERR_EVENT, msg);
                                }
                                continue;
                            };
                            let tt = (tm - base_shift - inst_lat(&slot)).max(now + 0.002);
                            // One voice per track, the same rule the rest of VOLT follows: whatever
                            // this lane was holding ends as the new note starts.
                            if let Some((old_slot, old_note)) = vst_held.insert(cell.track, (slot, cell.note)) {
                                if let Some(ov) = vsts.get(&old_slot) {
                                    ov.note_off(old_note, tt);
                                }
                            }
                            v.note_on(cell.note, cell.vel, tt);
                            continue;
                        }
                        let tt = (tm - base_shift).max(now + 0.002);
                        // Gxx — glide the HELD voice to this note instead of retriggering, the
                        // browser's exact rule (`ch.voice && !ch.voice.dead`). No voice to glide
                        // falls through to a normal trigger, exactly as schedRow does.
                        if cell.cmd == "G" {
                            if let Some(v) = live.get_mut(&cell.track) {
                                if v.can_glide() {
                                    let dur = clock.row_dur()
                                        * if cell.cmdv > 0.0 { (cell.cmdv / 16.0) as f64 } else { 1.0 };
                                    v.glide_to(cell.note, tt, dur);
                                    continue;
                                }
                            }
                        }
                        // The lane's previous note is cut BEFORE the new one is built, as the
                        // browser does it — so a failed trig still ends the note that was ringing
                        // rather than leaving it under a silence.
                        if let Some(mut old) = live.remove(&cell.track) {
                            old.cut(tt);
                            retired.push((old.quiet_by(tt), old));
                        }
                        let job = Job {
                            instr: cell.instr.clone(),
                            note: cell.note,
                            vel: cell.vel,
                            params: cell.params.clone(),
                            dur: clock.row_dur(),
                            sr,
                            t: tt,
                            rev: cell.rev,
                            // The strip is a parity-harness affordance; live playback routes
                            // through the mixer once stage C lands, not through the Job.
                            strip: None,
                            mix: None,
                        };
                        // A sampler note carries its audio in via the context; everything else
                        // uses the plain one. Building a borrowed Ctx per note is cheap — it is
                        // two references and an empty wavetable cache.
                        // `trig()` PANICS on a missing parameter — `Job::p()` is deliberately loud,
                        // because a silently-defaulted parameter is the one bug the parity harness
                        // could never catch by listening. That is right for a render binary and
                        // fatal here: an unwinding panic kills the audio thread, which drops the
                        // context, which releases the ASIO driver — and every command after it
                        // reports "the audio thread has stopped" with nothing to say why. One
                        // instrument in one song whose parameters do not match the port took the
                        // whole engine down permanently. Measured on Mickey's song, 2026-08-10.
                        //
                        // So the panic is caught and demoted to the same reported error a missing
                        // sample gets. `p()` stays loud where loud is useful.
                        let voice = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            match cell.sample.as_ref().and_then(|id| samples.get(id)) {
                                Some(buf) => trig(&Ctx::new(&ctx, &noise).with_sample(buf), &job),
                                None if cell.instr == "sampler" => {
                                    Err("sample not loaded on the engine yet".to_string())
                                }
                                None => trig(&c, &job),
                            }
                        }))
                        .unwrap_or_else(|e| {
                            let why = e
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_else(|| "panicked".into());
                            Err(why)
                        });
                        match voice {
                            Ok(v) => {
                                // Into this track's channel strip, not straight at the hardware.
                                // The strip is where the fader, EQ, filter, pan, drive and the two
                                // sends live — this one line is the difference between "the engine
                                // makes a noise" and "the engine has a mixer".
                                mixer.ensure(&ctx, cell.track + 1);
                                match mixer.input(cell.track) {
                                    Some(input) => v.connect(input),
                                    None => v.connect(&ctx.destination()),
                                }
                                live.insert(cell.track, v);
                                // Cxx / Uxx / Dxx land on whatever the lane now holds — the fresh
                                // voice, exactly as schedRow applies them after its trig block.
                                match cell.cmd.as_str() {
                                    "C" => {
                                        if let Some(v) = live.get(&cell.track) {
                                            // schedRow's curve: 40 Hz to 12 kHz, exponential
                                            let f = 40.0 * 300f32.powf(cell.cmdv / 255.0);
                                            v.set_filter_target(f, tt);
                                        }
                                    }
                                    "U" | "D" => {
                                        if let Some(v) = live.get_mut(&cell.track) {
                                            if !v.dead() {
                                                let sign = if cell.cmd == "U" { 1.0 } else { -1.0 };
                                                v.slide(sign * cell.cmdv * 12.5, tt, clock.row_dur());
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // A missing parameter is a bug in the caller, not a reason to kill the
                            // audio thread — report and keep the transport running. The UI hears
                            // about it once per distinct problem, so a looping pattern with one bad
                            // instrument does not become a notification every row.
                            Err(e) => {
                                let msg = format!("{}: {e}", cell.instr);
                                if said.insert(msg.clone()) {
                                    eprintln!("engine: {msg}");
                                    let _ = app.emit(ERR_EVENT, msg);
                                }
                            }
                        }
                    }
                }
            }

            // Let go of the voices whose tails have finished. Dropping them is what frees their
            // nodes — the render graph keeps a node alive as long as anything holds a handle, so
            // "the browser garbage-collects it" has no equivalent here and this is the collector.
            if !retired.is_empty() {
                let now = ctx.current_time();
                retired.retain(|(quiet, _)| *quiet > now);
            }

            // The playhead, going back the other way. Report the newest row whose time has arrived
            // and skip any it overtook — a UI cursor wants where playback *is*, not a replay of
            // every row it passed while the window was busy. The 5.8 ms of output latency between
            // this instant and the speaker is below the tick and is not compensated for.
            // **Is the engine handing the device something it cannot represent?**
            //
            // Checked while the transport runs, reported at most once a second. Everything else
            // in this file measures TIME — render load, underruns, plugin blocks — and all of it
            // stayed clean while Mickey heard distortion on every note (2026-08-14). Level is the
            // other half: a master above 1.0 is squared off by the converter, which is harsh, is
            // shaped exactly like the notes, and is invisible to every CPU meter there is.
            if busy && clock.playing() {
                let now_ms = ctx.current_time();
                if now_ms - last_peak_log > 1.0 {
                    let p = mixer.master_peak(&mut meter_scratch);
                    if p > 0.999 {
                        crate::log!("audio: MASTER IS CLIPPING — peak {p:.3} (over full scale; the converter squares this off)");
                        last_peak_log = now_ms;
                    } else if p > 0.9 {
                        crate::log!("audio: master peak {p:.3} — close to full scale");
                        last_peak_log = now_ms;
                    }
                }
            }

            if busy {
                let now = ctx.current_time();
                let mut cur = None;
                while pending.front().is_some_and(|(_, t)| *t <= now) {
                    cur = pending.pop_front().map(|(r, _)| r);
                }
                match cur {
                    Some(r) if r as i64 != last_row => {
                        last_row = r as i64;
                        let _ = app.emit(ROW_EVENT, last_row);
                    }
                    _ => {}
                }
                // Nothing playing, nothing left to sound: tell the UI to put the cursor away.
                if !clock.playing() && pending.is_empty() && last_row >= 0 && now >= tail_t {
                    last_row = -1;
                    let _ = app.emit(ROW_EVENT, -1i64);
                }
                std::thread::sleep(std::time::Duration::from_secs_f64(TICK));
            }
        }

        // **Take every plugin's patch with us.**
        //
        // This device's plugins are about to cease to exist, and the ones that replace them are
        // born at their factory preset. Asking now — while the render thread is still turning, so
        // the message round-trip has someone to answer it — is what makes a reopen invisible.
        // Timed out per plugin rather than in total: one sulking plugin costs its own patch, not
        // everybody else's.
        for (slot, node) in vsts.iter() {
            let key = format!("i{slot}");
            if let Some(ident) = plugins::ident_for(&key) {
                if let Some(bytes) = node.state(250) {
                    crate::log!("patch kept: {key} ({} bytes)", bytes.len());
                    plugins::remember_patch(key, ident, bytes);
                }
            }
        }
        for ((track, pos), node) in fx_vsts.iter() {
            let key = format!("f{track}.{pos}");
            if let Some(ident) = plugins::ident_for(&key) {
                if let Some(bytes) = node.state(250) {
                    crate::log!("patch kept: {key} ({} bytes)", bytes.len());
                    plugins::remember_patch(key, ident, bytes);
                }
            }
        }

        // **Close the context. Do not merely drop it.**
        //
        // Dropping leaks the running backend on purpose (see PREV_CLOSED) — the stream plays on
        // with nothing left that can address it, which is precisely the sound a panic cannot kill.
        // Everything audible must die here, in this order: notes and plugin voices first (so the
        // last blocks the driver renders are silent rather than a chord being cut mid-air), then
        // the device.
        cut_all(&mut live, &mut retired, ctx.current_time());
        vst_all_off(&vsts, &mut vst_held, ctx.current_time());
        for v in vsts.values() {
            v.all_off();
        }
        retired.clear();
        live.clear();
        crate::log!("audio thread: closing the device");
        ctx.close_sync();
        crate::log!("audio thread: device closed and released");
        // Whoever is waiting to open the next device can go ahead now.
        let _ = closed_tx.send(());
    });

    // Fifteen seconds. It was eight, on the belief that "a driver that is going to open opens in
    // well under a second" — which is true of WASAPI and false of ASIO: **ASIO4ALL measures 1.9 s
    // from a spawned thread on this machine**, and that is with nothing else going on. A budget
    // that tight turned a slow-but-fine open into an error message blaming other software, which
    // cost an evening of looking for a DAW that was not running. Long enough for a real driver,
    // short enough that a driver which will never answer does not freeze the window for good.
    let opened = ready_rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .map_err(|_| {
            // The command in this message named `volt-desktop`, and the running process is not
            // always called that — which sent a real debugging session chasing a process that
            // "did not exist" while it was in fact running the whole time. A wildcard is right
            // here: the user does not care what the binary is called, only that VOLT is gone.
            "that driver did not respond within 15 s. Something else is probably holding it — \
             another DAW, or a VOLT that did not shut down cleanly (ASIO4ALL is prone to it). \
             Close them, or run  Get-Process *volt* | Stop-Process -Force  in PowerShell, \
             and try again."
                .to_string()
        })??;
    // Publish the new engine only now that it is genuinely up, and remember how to wait for it
    // when the NEXT device takes over.
    *PREV_CLOSED.lock().unwrap_or_else(|e| e.into_inner()) = Some(closed_rx);
    *state.0.lock().map_err(|_| "engine lock poisoned")? = Some(tx);
    Ok(opened)
}

/// Hand the engine one decoded sample, to be referenced by id from then on.
///
/// `pcm` is interleaved f32 already at the device's sample rate — the webview resamples, because
/// it has an `OfflineAudioContext` that does it correctly and this side would need a resampler
/// nobody has measured. Sent once per sample, not per note: a pattern crosses on every play.
#[tauri::command(async)]
pub fn eng_load_sample(
    state: tauri::State<'_, Engine>,
    id: String,
    pcm: Vec<f32>,
    channels: usize,
) -> Result<usize, String> {
    let slot = state.0.lock().map_err(|_| "engine lock poisoned")?;
    let tx = slot.as_ref().ok_or("no audio device open — call eng_open first")?;
    let frames = pcm.len() / channels.max(1);
    tx.send(Cmd::Sample { id, pcm, channels })
        .map_err(|_| "the audio thread has stopped".to_string())?;
    Ok(frames)
}

#[tauri::command(async)]
pub fn eng_play(state: tauri::State<'_, Engine>, pattern: Pattern) -> Result<(), String> {
    let slot = state.0.lock().map_err(|_| "engine lock poisoned")?;
    let tx = slot.as_ref().ok_or("no audio device open — call eng_open first")?;
    tx.send(Cmd::Play(pattern))
        .map_err(|_| "the audio thread has stopped".to_string())
}

/// The contract with the webview, pinned.
///
/// Everything either side of the IPC boundary is tested — `tests/nengine.test.js` runs the palette
/// commands and asserts the payloads, the Rust commands have their own tests — but the boundary
/// itself was not, because WebView2 under Tauri release refuses a remote-debugging port and the
/// real webview cannot be driven headlessly.
///
/// This closes that. The JSON below is **copied verbatim from what the JS test captured**, so if
/// either side renames a field, changes a type, or starts sending something serde will not accept,
/// this fails — which is the entire remaining class of bug at that boundary. Keep it in sync by
/// pasting, not by editing to taste.
#[cfg(test)]
mod tests {
    use super::*;

    /// Exactly what `nengPattern()` produced in tests/nengine.test.js, srcInst already stripped.
    const FROM_JS: &str = r#"{
      "bpm":170,"lpb":4,"rows":64,"repeats":1,
      "cells":[
        {"row":0,"track":0,"instr":"sampler","note":60,"vel":1,
         "params":{"s0":0.1,"s1":0.9,"pad":0,"off":0},"sample":"hash-abc","tx":0},
        {"row":2,"track":0,"instr":"sampler","note":60,"vel":0.6299212598425197,
         "params":{"s0":0.1,"s1":0.9,"pad":0,"off":0.5019607843137255},"sample":"hash-abc","tx":0},
        {"row":6,"track":0,"instr":"","note":-1,"vel":0,"params":{},"sample":null,"tx":0},
        {"row":4,"track":1,"instr":"kick","note":48,"vel":1,
         "params":{"pitch":50,"punch":0.6,"decay":0.4,"drive":0.3},"sample":null,"tx":0}
      ]}"#;

    #[test]
    fn the_webviews_pattern_deserialises() {
        let p: Pattern = serde_json::from_str(FROM_JS).expect("JS payload rejected by serde");
        assert_eq!(p.bpm, 170.0);
        assert_eq!(p.rows, 64);
        assert_eq!(p.cells.len(), 4);

        let s = &p.cells[0];
        assert_eq!(s.instr, "sampler");
        assert_eq!(s.track, 0);
        assert_eq!(s.sample.as_deref(), Some("hash-abc"));
        assert_eq!(s.params.get("s0"), Some(&0.1));
        // Sxx arrives as a fraction of the trim region, not as the raw 0..255 byte.
        assert!((p.cells[1].params["off"] - 0.501_960_8).abs() < 1e-6);
        // The note-OFF: no instrument, no params, and the lane it has to release.
        let off = &p.cells[2];
        assert!(off.note < 0.0, "a note-OFF must arrive as a negative note");
        assert_eq!(off.track, 0);
        assert!(off.instr.is_empty() && off.params.is_empty());
        // A non-sampler note sends `null`, which must land as None rather than failing.
        assert!(p.cells[3].sample.is_none());
        assert_eq!(p.cells[3].track, 1, "the kick is on its own lane");
    }

    #[test]
    fn a_note_with_only_the_essentials_still_loads() {
        // Every #[serde(default)] field omitted — what a minimal caller would send. If one of them
        // loses its default this stops compiling into a valid parse and the app breaks on a note
        // that simply had no Txx.
        let one: Pattern = serde_json::from_str(
            r#"{"bpm":120,"lpb":4,"rows":16,"cells":[{"row":0,"instr":"kick","note":48,"vel":1}]}"#,
        )
        .expect("minimal payload rejected");
        assert_eq!(one.repeats, 1, "repeats should default to one pass");
        assert_eq!(one.cells[0].tx, 0);
        assert!(one.cells[0].sample.is_none());
        assert!(one.cells[0].params.is_empty());
    }

    #[test]
    fn a_vst_cell_crosses_intact() {
        // The one failure mode left on this wire is a serde mismatch — a field the UI sends under a
        // name Rust does not read, which does not error, it just silently becomes None and the note
        // goes nowhere. `tests/nengine.test.js` asserts the JS side produces exactly this shape;
        // this asserts Rust reads it. The two together are the whole contract.
        let p: Pattern = serde_json::from_str(
            r#"{"bpm":120,"lpb":4,"rows":16,"cells":[
                 {"row":0,"track":1,"instr":"vst","vst":3,"note":60,"vel":0.78,"params":{}},
                 {"row":4,"track":1,"instr":"","note":-1,"vel":0}]}"#,
        )
        .expect("a vst cell must deserialise");
        assert_eq!(p.cells[0].instr, "vst");
        assert_eq!(p.cells[0].vst, Some(3), "the slot must survive the crossing");
        assert_eq!(p.cells[0].track, 1);
        // A note-OFF carries no instrument and no slot: the engine finds what to stop by lane.
        assert!(p.cells[1].note < 0.0);
        assert_eq!(p.cells[1].vst, None);
    }

    #[test]
    fn an_aligned_start_crosses_and_absence_means_the_old_default() {
        // The JS side sends start_in/from_row for the flam fix; an old payload without them must
        // behave exactly as before (0 → the fixed 0.15, row 0). Both directions matter: the field
        // names are the contract, and serde makes a typo'd name a silent None, not an error.
        let p: Pattern = serde_json::from_str(
            r#"{"bpm":174,"lpb":4,"rows":16,"cells":[],"start_in":0.228,"from_row":6}"#,
        ).unwrap();
        assert!((p.start_in - 0.228).abs() < 1e-9);
        assert_eq!(p.from_row, 6);
        let old: Pattern = serde_json::from_str(r#"{"bpm":174,"lpb":4,"rows":16,"cells":[]}"#).unwrap();
        assert_eq!(old.start_in, 0.0);
        assert_eq!(old.from_row, 0);
    }

    #[test]
    fn duck_triggers_cross_and_absence_means_none() {
        // The sidechain triggers ride NEXT TO the cells (see DuckTrig for why), in the shape
        // nengPattern() builds. A payload from an older UI has no `ducks` and must mean an empty
        // list, not a parse error.
        let p: Pattern = serde_json::from_str(
            r#"{"bpm":140,"lpb":4,"rows":16,"cells":[],
                "ducks":[{"row":0,"src":1,"tx":0,"kick":true},
                         {"row":8,"src":3,"tx":64,"kick":false}]}"#,
        )
        .expect("ducks must deserialise");
        assert_eq!(p.ducks.len(), 2);
        assert!(p.ducks[0].kick);
        assert_eq!(p.ducks[1].tx, 64, "an off-grid trigger keeps its Txx");
        let old: Pattern =
            serde_json::from_str(r#"{"bpm":140,"lpb":4,"rows":16,"cells":[]}"#).unwrap();
        assert!(old.ducks.is_empty());
    }

    #[test]
    fn voice_commands_cross_and_absence_is_a_plain_note() {
        // Gxx/Cxx/Uxx/Dxx ride the cell as `cmd`+`cmdv`; a payload without them (every cell from
        // before 2026-08-13, and every plain note after) must mean "no command", not a parse error.
        let p: Pattern = serde_json::from_str(
            r#"{"bpm":174,"lpb":4,"rows":16,"cells":[
                 {"row":0,"track":4,"instr":"reese","note":24,"vel":1,"params":{},"cmd":"G","cmdv":8},
                 {"row":2,"track":4,"instr":"reese","note":24,"vel":1,"params":{},"cmd":"C","cmdv":128},
                 {"row":4,"track":4,"instr":"reese","note":24,"vel":1,"params":{}}]}"#,
        )
        .expect("command cells must deserialise");
        assert_eq!(p.cells[0].cmd, "G");
        assert_eq!(p.cells[0].cmdv, 8.0);
        assert_eq!(p.cells[1].cmd, "C");
        assert_eq!(p.cells[2].cmd, "", "no command must arrive as the empty string");
    }

    #[test]
    fn a_cell_with_no_plugin_is_not_a_plugin_cell() {
        // `vst` absent must mean None, not 0 — a default of 0 would send every ordinary note to
        // whatever plugin happened to be in slot 0.
        let p: Pattern = serde_json::from_str(
            r#"{"bpm":120,"lpb":4,"rows":16,"cells":[{"row":0,"instr":"kick","note":48,"vel":1}]}"#,
        )
        .unwrap();
        assert_eq!(p.cells[0].vst, None);
    }

    #[test]
    fn an_unknown_field_is_ignored_not_fatal() {
        // serde ignores unknown fields by default, and that is what we want: the JS side strips
        // `srcInst`, but a future field arriving from a newer UI must not brick playback.
        let p: Pattern = serde_json::from_str(
            r#"{"bpm":120,"lpb":4,"rows":16,"cells":[
                 {"row":0,"instr":"kick","note":48,"vel":1,"srcInst":3,"somethingNew":true}]}"#,
        )
        .expect("unknown field should be ignored");
        assert_eq!(p.cells.len(), 1);
    }
}

#[tauri::command(async)]
pub fn eng_stop(state: tauri::State<'_, Engine>) -> Result<(), String> {
    let slot = state.0.lock().map_err(|_| "engine lock poisoned")?;
    if let Some(tx) = slot.as_ref() {
        let _ = tx.send(Cmd::Stop);
    }
    Ok(())
}

/// The panic, engine half: stop the clock, cut every voice, all-notes-off into every plugin —
/// and WAIT for the audio thread to say it did it.
///
/// The wait is the difference between a panic and a wish. The audio thread has wedged before
/// (2026-08-11: ten `vst_load`s in a row timed out), and a panic that quietly posts to a wedged
/// thread leaves the user with the one thing a panic must never leave: sound, continuing. On
/// timeout this returns an error naming the state, and the UI escalates to `eng_close`.
#[tauri::command(async)]
pub fn eng_panic(state: tauri::State<'_, Engine>) -> Result<(), String> {
    let (reply, ack) = channel();
    {
        let slot = state.0.lock().map_err(|_| "engine lock poisoned")?;
        let Some(tx) = slot.as_ref() else {
            return Ok(()); // no device open — the native side has nothing sounding
        };
        tx.send(Cmd::Panic { reply })
            .map_err(|_| "the audio thread has stopped")?;
    }
    // Generous against a busy tick (10 ms sleep per loop, one command per pass), tight enough
    // that a wedged thread is called wedged while the user is still holding Alt+P.
    ack.recv_timeout(std::time::Duration::from_millis(800))
        .map_err(|_| "the audio thread is not answering".to_string())
}

/// Drop the engine: the device closes, the driver is released, all native sound ends.
///
/// The nuclear half of panic, and also just "turn the engine off". Dropping the sender ends the
/// audio thread the moment it next looks at its channel; a wedged thread cannot look, but nothing
/// can reach a wedged thread anyway — closing at least frees every future command from queueing
/// behind it, and the UI can offer a reopen.
#[tauri::command(async)]
pub fn eng_close(state: tauri::State<'_, Engine>) -> Result<(), String> {
    let mut slot = state.0.lock().map_err(|_| "engine lock poisoned")?;
    *slot = None;
    // No device, no health to report — and no monitor of a dying context allowed to write here.
    STATS_GEN.store(u64::MAX, std::sync::atomic::Ordering::SeqCst);
    *ENG_STATS.lock().unwrap_or_else(|e| e.into_inner()) = EngStats::default();
    Ok(())
}

/// One meter reading: every strip's peak, plus the spectrum of `chan` (or the master when -1).
///
/// Polled by the UI at drawing rate while a plugin channel is on screen — the sound it wants to
/// draw lives in this graph, where the browser's AnalyserNodes cannot see it.
#[tauri::command(async)]
pub fn eng_meters(state: tauri::State<'_, Engine>, chan: i64) -> Result<MeterData, String> {
    let (reply, answer) = channel();
    {
        let slot = state.0.lock().map_err(|_| "engine lock poisoned")?;
        let tx = slot.as_ref().ok_or("no audio device open")?;
        tx.send(Cmd::Meters { chan: usize::try_from(chan).ok(), reply })
            .map_err(|_| "the audio thread has stopped")?;
    }
    answer
        .recv_timeout(std::time::Duration::from_millis(400))
        .map_err(|_| "the audio thread is not answering".to_string())
}

/// The plugin list, cached for the life of the process and on disk between runs.
///
/// A scan opens every plugin file on the machine and asks it to identify itself. That is seconds of
/// work and it is the same answer every time until something is installed, so paying for it once
/// per session — in the background, at startup — is the difference between a DAW and a tool that
/// makes you wait to pick a synth.
static SCAN_CACHE: Mutex<Option<VstScan>> = Mutex::new(None);

/// Where the disk cache lives. Next to the songs, not in the repo: it describes THIS machine.
///
/// The number is bumped whenever a cached scan would be WRONG rather than merely old, because a
/// cache is trusted on sight and nothing else would ever throw it away. `-2`: VST2 entries and the
/// skipped count (2026-08-10). `-3`: the duplicate-class fix (2026-08-11) — every `-2` cache on
/// disk lists each VST3 twice, and half of those entries cannot be loaded. `-4`: effects joined
/// the list with a `kind` field (2026-08-12) — a `-3` cache holds instruments only, so the FX
/// picker would read it as "you own no effects".
fn scan_cache_path() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)?;
    Some(dir.join("VOLT").join("vst-scan-4.json"))
}

/// The journal of the scan in progress: each file's path is written (and flushed) BEFORE that file
/// is opened, and the journal is deleted when the scan completes. So if the journal exists at the
/// start of a scan, its last line names a plugin that ended the previous scan without finishing —
/// either it crashed the process mid-open, or it HUNG it (Bionic Delay hangs in `LoadLibrary`
/// forever, measured) and the app was closed around it. Since the scan runs at startup, that
/// plugin would otherwise wreck *every* startup, with the cache never written. Its file goes on
/// the blacklist instead, permanently. This became worth having the day the scan grew from 129
/// curated VST3s to every DLL in three VST2 folders. (The blacklist itself lives in
/// `volt_vsthost`, shared with `vstprobe` — see `blacklist_path` there.)
fn scan_journal_path() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)?;
    Some(dir.join("VOLT").join("vst-scan-journal.txt"))
}

/// Every VST instrument on this machine — VST3 and 64-bit VST2 in one list.
///
/// Deliberately NOT on the audio thread: a scan opens a few hundred DLLs and takes seconds, and
/// the audio thread has a callback to feed.
#[tauri::command(async)]
pub fn vst_scan() -> VstScan {
    if let Ok(c) = SCAN_CACHE.lock() {
        if let Some(scan) = c.as_ref() {
            return scan.clone();
        }
    }
    // A cache from a previous run. Trusted on sight: the worst case is a plugin that has been
    // uninstalled since, which fails to load with a message naming it, rather than a wrong sound.
    if let Some(path) = scan_cache_path() {
        if let Ok(txt) = std::fs::read_to_string(&path) {
            if let Ok(scan) = serde_json::from_str::<VstScan>(&txt) {
                if !scan.plugins.is_empty() {
                    if let Ok(mut c) = SCAN_CACHE.lock() {
                        *c = Some(scan.clone());
                    }
                    return scan;
                }
            }
        }
    }
    vst_rescan()
}

/// Scan for real, ignoring every cache, and write the result to disk. This is what runs at startup
/// in the background, and what "rescan" would call after installing a plugin.
#[tauri::command(async)]
pub fn vst_rescan() -> VstScan {
    let out = vst_scan_now();
    if let Ok(mut c) = SCAN_CACHE.lock() {
        *c = Some(out.clone());
    }
    if let Some(path) = scan_cache_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(txt) = serde_json::to_string(&out) {
            let _ = std::fs::write(&path, txt);
        }
    }
    out
}

fn vst_scan_now() -> VstScan {
    use std::io::Write;

    // A journal left behind means the last scan died — or hung and was closed — inside its
    // last-named file. Blacklist that file for good; the alternative is an app that fails the
    // same way at every startup until someone works out which of 300 DLLs is the killer.
    if let Some(jp) = scan_journal_path() {
        if let Ok(txt) = std::fs::read_to_string(&jp) {
            if let Some(last) = txt.lines().rev().find(|l| !l.is_empty()) {
                if volt_vsthost::blacklist(std::path::Path::new(last)) {
                    eprintln!("vst scan: {last} ended the last scan — blacklisted");
                }
            }
        }
    }
    let blacklist = volt_vsthost::read_blacklist();
    let mut journal = scan_journal_path().and_then(|p| {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::File::create(p).ok()
    });
    let mut about_to_open = |path: &std::path::Path| {
        if let Some(f) = journal.as_mut() {
            // The flush is the entire point: an unflushed line dies with the process.
            let _ = writeln!(f, "{}", path.display());
            let _ = f.flush();
        }
    };
    let banned =
        |path: &std::path::Path| blacklist.iter().any(|b| b.as_str() == path.to_string_lossy());

    let mut plugins = Vec::new();
    for path in volt_vsthost::scan(std::path::Path::new(volt_vsthost::VST3_DIR)) {
        if banned(&path) {
            continue;
        }
        about_to_open(&path);
        let Ok(module) = volt_vsthost::Module::open(&path) else { continue };
        for c in module.classes() {
            // **Audio classes only, and this is not cosmetic.** A VST3 file exports its audio
            // module AND its component-controller class, both under the same name and the same
            // subcategories — so filtering on `is_instrument()` alone listed every plugin on the
            // machine TWICE, and the second copy could not be loaded at all: it is not an
            // `IComponent`, so `createInstance` returns E_NOINTERFACE. Picking the wrong Vital of
            // the two was a coin flip. Found by `examples/editwin.rs` 2026-08-11.
            if !c.category.contains("Audio Module") {
                continue;
            }
            // Instruments AND effects, since 2026-08-12 — the mixer has an insert for a plugin
            // effect to sit in (`Mixer::set_plugin_insert`), so listing them stopped being an
            // offer of something unusable. `kind` is what the two pickers filter on.
            plugins.push(VstInfo {
                name: c.name.clone(),
                vendor: c.vendor.clone(),
                path: c.path.to_string_lossy().to_string(),
                index: c.index,
                fmt: "VST3".to_string(),
                kind: if c.is_instrument() { "inst" } else { "fx" }.to_string(),
            });
        }
    }

    // The VST2 folders. `scan_vst2` has already set the 32-bit DLLs aside and counted them; that
    // count rides along to the UI, because "half my plugins are missing" and "114 skipped: 32-bit"
    // are very different experiences of the same scan.
    let v2 = volt_vsthost::scan_vst2();
    for path in &v2.dlls {
        if banned(path) {
            continue;
        }
        about_to_open(path);
        let Ok(module) = volt_vsthost::Module::open(path) else { continue };
        for c in module.classes() {
            plugins.push(VstInfo {
                name: c.name.clone(),
                vendor: c.vendor.clone(),
                path: c.path.to_string_lossy().to_string(),
                index: c.index,
                fmt: "VST2".to_string(),
                kind: if c.is_instrument() { "inst" } else { "fx" }.to_string(),
            });
        }
    }

    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    // The scan survived every file it touched; the journal has nothing left to say.
    drop(journal);
    if let Some(jp) = scan_journal_path() {
        let _ = std::fs::remove_file(jp);
    }
    VstScan { plugins, skipped32: v2.skipped_32bit }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct VstInfo {
    pub name: String,
    pub vendor: String,
    pub path: String,
    pub index: i32,
    /// "VST3" or "VST2" — for the picker's label and nothing else. The load path does not read
    /// it; `Module::open` decides from the file.
    #[serde(default)]
    pub fmt: String,
    /// "inst" or "fx" — which picker offers it. An empty value (a cache written before effects
    /// were listed) reads as an instrument, which is all such a cache could contain.
    #[serde(default)]
    pub kind: String,
}

/// What a scan hands the UI: the list, plus the count of 32-bit VST2s that were seen and cannot
/// load in a 64-bit process. Carried instead of buried in a log so the picker can say so.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct VstScan {
    pub plugins: Vec<VstInfo>,
    #[serde(default)]
    pub skipped32: usize,
}

// Plugins — creation, destruction and windows — all live on their own thread now. See
// `crate::plugins` for why neither the audio thread nor the main thread was allowed to be it.
use crate::plugins;

/// Load a plugin into an instrument slot. Returns the plugin's own name for the status line.
///
/// **The plugin is created on the main thread**, then moved to the audio thread to be wired into
/// the graph. That order is not stylistic: see `create_plugin`. The cost is that a slow plugin
/// (Kontakt is about a second) briefly blocks the window instead of briefly blocking audio, which
/// is the trade every DAW makes when you add a plugin.
#[tauri::command(async)]
pub fn vst_load(
    state: tauri::State<'_, Engine>,
    slot: usize,
    path: String,
    index: i32,
) -> Result<String, String> {
    let sample_rate = {
        let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
        guard.as_ref().ok_or("no audio device open — open one first")?;
        // The engine reports its rate when it opens; the plugin has to be set up for the same one.
        *SAMPLE_RATE.lock().unwrap_or_else(|e| e.into_inner())
    };
    if sample_rate <= 0.0 {
        return Err("no audio device open — open one first".into());
    }

    // Create on the PLUGIN thread — not the audio thread (no message pump, VST2 editors hang) and
    // not the main thread (this call takes about a second for a big sampler, and the window would
    // stop painting for all of it).
    crate::log!("vst_load: slot {slot} <- {path}");
    // The patch this slot was last known to hold travels WITH the create request and is applied
    // on the plugin thread — see `plugins::create`. Doing it here, on a Tauri worker, crashed
    // apQualizr2 inside ntdll (2026-08-15). Wrong-plugin states can't arrive: keyed by identity.
    let patch = plugins::patch_for(&format!("i{slot}"), &format!("{path}|{index}"));
    let (plugin, editor) = plugins::create_plugin(sample_rate, path.clone(), index, patch)
        .inspect_err(|e| crate::log!("vst_load: slot {slot} create failed: {e}"))?;
    // The parameter surface, captured while the plugin is still reachable from this thread —
    // the automation picker reads it from the registry, never from the plugin.
    remember_params(format!("i{slot}"), &plugin.param_list);
    remember_tag(plugin.tag, format!("i{slot}"));
    plugins::remember_ident(format!("i{slot}"), format!("{path}|{index}"));

    // Then hand the audio half to the audio thread.
    let (reply, answer) = channel();
    {
        let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
        let tx = guard.as_ref().ok_or("no audio device open — open one first")?;
        tx.send(Cmd::VstAttach { slot, plugin: Box::new(plugin), reply })
            .map_err(|_| "the audio thread has stopped")?;
    }
    let name = answer
        .recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|_| {
            crate::log!("vst_load: slot {slot} — the AUDIO THREAD never answered in 30 s");
            "the plugin did not reach the audio engine".to_string()
        })?
        .inspect_err(|e| crate::log!("vst_load: slot {slot} the engine refused it: {e}"))?;
    crate::log!("vst_load: slot {slot} is playing {name}");

    // Replacing a slot's plugin drops the old editor — and with it the old window, which would
    // otherwise sit on screen belonging to a plugin that no longer exists.
    match editor {
        Some(h) => plugins::adopt_editor(slot, h),
        None => plugins::forget(slot),
    }
    Ok(name)
}

/// The open device's sample rate, so a plugin can be created for it away from the audio thread.
static SAMPLE_RATE: Mutex<f64> = Mutex::new(0.0);

/// The last mixer snapshot the UI sent.
///
/// Kept so a device opened *after* the song was loaded — or switched mid-session — comes up with
/// the song's actual faders rather than defaults. Without it, choosing ASIO would silently reset
/// everybody's mix to unity, which is the kind of bug that sounds like the mixer not working.
static MIX_STATE: Mutex<Option<MixState>> = Mutex::new(None);

/// Push the whole mixer state to the engine. `applyMix()`'s counterpart across the wire.
///
/// Wholesale on every edit, deliberately: the browser re-applies every parameter of every channel
/// on any mixer change, it costs microseconds there and the same here, and it means the engine's
/// graph cannot drift out of step with the song. A delta protocol would be smaller and would
/// eventually disagree with the UI, which is the failure that matters.
#[tauri::command(async)]
pub fn eng_mix(state: tauri::State<'_, Engine>, mix: MixState) -> Result<(), String> {
    // Everything below happens under one lock, which is what makes the ordering guarantee hold:
    // the sequence check and the send cannot interleave with another update.
    let mut latest = MIX_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if mix.seq > 0 {
        if let Some(prev) = latest.as_ref() {
            // **A LOWER sequence number means the page restarted, not that this is stale.**
            //
            // The counter lives in the webview and starts at 1 there; the engine outlives a page
            // reload (Ctrl+R reloads the page, the Rust side keeps running). So after one reload
            // the UI sends seq 1, 2, 3 while this remembers 400 — and every mixer edit from then
            // on was dropped in silence. Symptom: on the channels the ENGINE owns (plugin
            // instruments, VST-FX channels) the pan knob and the faders did nothing at all, while
            // browser-owned channels obeyed instantly — reported 2026-08-15 as "the mixer panning
            // not affecting channels with fx or vst inst over them", and the engine's own panning
            // measured perfect the whole time (examples/panprobe).
            //
            // Out-of-order really does happen (commands run on a thread pool), so the guard stays
            // — but it only rejects a snapshot that is BEHIND BY A LITTLE, which is what an
            // overtaken message looks like. A big backwards jump is a new page, and the newest
            // thing the UI has said is by definition current.
            let overtaken = mix.seq <= prev.seq && prev.seq - mix.seq < 64;
            if overtaken {
                // An older snapshot that overtook a newer one on the thread pool. Drop it whole —
                // the state is wholesale, so the newer one already says everything this one would.
                return Ok(());
            }
            if mix.seq <= prev.seq {
                crate::log!(
                    "eng_mix: sequence restarted ({} after {}) — the page reloaded; following it",
                    mix.seq, prev.seq
                );
            }
        }
    }
    *latest = Some(mix.clone());
    // **`try_lock`, never `lock`.** `eng_open` holds this mutex for as long as a driver takes to
    // start — up to 15 seconds for one that never answers — and a mixer edit arriving in that
    // window would block a Tauri worker for the same 15 seconds. Enough of those and every command
    // the UI sends is stuck behind a device that is still starting.
    //
    // Skipping is free here, and only here, because the mixer state is sent WHOLESALE: whatever is
    // dropped, the next message carries in full, and the snapshot above means a device that opens
    // later inherits it anyway. A delta protocol could not do this.
    match state.0.try_lock() {
        Ok(guard) => {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(Cmd::Mix(Box::new(mix)));
            }
        }
        Err(_) => { /* engine busy opening — the snapshot above is what it will read */ }
    }
    Ok(())
}

#[tauri::command(async)]
pub fn vst_unload(state: tauri::State<'_, Engine>, slot: usize) -> Result<(), String> {
    // The window goes BEFORE the plugin does. Its view is drawn by code inside the plugin, so
    // tearing the plugin down first would leave a window whose contents belong to a released
    // object — and closing it later would call into it.
    plugins::forget(slot);
    // Taking a plugin off a slot is a decision, not an accident — its patch goes with it, so
    // putting the same plugin back later starts clean rather than in someone's forgotten state.
    plugins::drop_patch(&format!("i{slot}"));

    let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(Cmd::VstUnload { slot });
    }
    // Nothing to schedule after this: the audio thread posts the plugin down the retire channel
    // and the plugin thread destroys it on its next pass, a few milliseconds later.
    Ok(())
}

/// Open (or raise) the plugin's own window for one instrument slot.
///
/// Runs on the plugin thread, which is the only thread allowed to touch a plugin's window — see
/// `crate::plugins`. This blocks a Tauri worker until the window is up, which is right: the caller
/// wants to know whether it worked. It does not block the UI.
#[tauri::command(async)]
pub fn vst_editor_open(slot: usize) -> Result<(), String> {
    plugins::editor_open(slot)
}

/// Hide a plugin's window. Deliberately hide, not destroy — see `editor.rs`: one view per plugin
/// for its whole life, so re-opening is instant and no plugin is ever asked for a second one.
#[tauri::command(async)]
pub fn vst_editor_close(slot: usize) -> Result<(), String> {
    plugins::editor_close(slot);
    Ok(())
}

// ---- plugin EFFECTS on mixer channels ----------------------------------------------------------
//
// The same machinery as instruments — created on the plugin thread, attached on the audio thread,
// window on the plugin thread — with two differences: the key is a mixer TRACK, and the node goes
// into the strip's insert (`Mixer::set_plugin_insert`) instead of being played by notes. This is
// ROADMAP step 2 ("One VST3 effect in the strip — Pro-Q 3 on a channel, heard through ASIO"), and
// it is what makes the Waves catalogue usable at all: WaveShell VST3 is almost entirely effects.

/// Put a plugin effect on a mixer channel (`track` >= 0) or on the MASTER bus (`track` = -1).
/// Returns the plugin's name for the panel.
#[tauri::command(async)]
pub fn fxvst_load(
    app: tauri::AppHandle,
    state: tauri::State<'_, Engine>,
    track: i64,
    pos: Option<usize>,
    path: String,
    index: i32,
) -> Result<String, String> {
    // `pos` places the plugin in the channel's CHAIN (0 first). Optional so an older payload
    // means what it always meant: the only slot.
    let pos = pos.unwrap_or(0);
    if pos >= FX_CHAIN_MAX {
        return Err(format!("chain position {pos} is past the limit of {FX_CHAIN_MAX}"));
    }
    let sample_rate = {
        let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
        guard.as_ref().ok_or("no audio device open — open one first")?;
        *SAMPLE_RATE.lock().unwrap_or_else(|e| e.into_inner())
    };
    if sample_rate <= 0.0 {
        return Err("no audio device open — open one first".into());
    }

    crate::log!("fxvst_load: track {track} pos {pos} <- {path}");
    // Same rule as `vst_load`: the patch is applied on the plugin thread, not here.
    let patch = plugins::patch_for(&format!("f{track}.{pos}"), &format!("{path}|{index}"));
    let (plugin, editor) = plugins::create_plugin(sample_rate, path.clone(), index, patch)
        .inspect_err(|e| crate::log!("fxvst_load: track {track} pos {pos} create failed: {e}"))?;
    if plugin.in_channels == 0 {
        // An instrument in an insert would mute the whole channel: with no input bus it cannot
        // pass the strip's audio through, so everything upstream of it simply disappears.
        return Err(format!("{} has no audio input — it is an instrument, not an effect", plugin.name));
    }
    // Noted before the plugin moves to the audio thread, where it can no longer be asked.
    let mono_fx = plugin.out_channels < 2 || plugin.in_channels < 2;
    remember_params(format!("f{track}.{pos}"), &plugin.param_list);
    plugins::remember_ident(format!("f{track}.{pos}"), format!("{path}|{index}"));
    remember_tag(plugin.tag, format!("f{track}.{pos}"));

    let (reply, answer) = channel();
    {
        let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
        let tx = guard.as_ref().ok_or("no audio device open — open one first")?;
        tx.send(Cmd::FxVstAttach { track, pos, plugin: Box::new(plugin), reply })
            .map_err(|_| "the audio thread has stopped")?;
    }
    let name = answer
        .recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|_| {
            crate::log!("fxvst_load: track {track} pos {pos} — the AUDIO THREAD never answered in 30 s");
            "the plugin did not reach the audio engine".to_string()
        })?
        .inspect_err(|e| crate::log!("fxvst_load: track {track} pos {pos} the engine refused it: {e}"))?;
    crate::log!("fxvst_load: track {track} pos {pos} is running {name}");
    // **Say it when a plugin is MONO.** A mono instance folds whatever reaches it to one channel
    // and hands back one channel — on the master that is the entire mix, image gone, and it looks
    // like VOLT broke rather than like a plugin doing exactly what its name says. (It no longer
    // THROWS AWAY the right side — see the downmix in vsthost's `process_io` — but mono is still
    // mono.) Waves ships a Stereo class of nearly everything; naming it is the whole fix.
    if mono_fx {
        let where_ = if track < 0 { "the master".to_string() } else { format!("channel {}", track + 1) };
        let msg = format!(
            "{name} is a MONO plugin — it collapses {where_} to mono. Use the Stereo version to keep the image."
        );
        crate::log!("fxvst_load: {msg}");
        let _ = app.emit(ERR_EVENT, msg);
    }

    match editor {
        Some(h) => plugins::adopt_editor(fx_editor_slot(track, pos), h),
        None => plugins::forget(fx_editor_slot(track, pos)),
    }
    Ok(name)
}

#[tauri::command(async)]
pub fn fxvst_unload(state: tauri::State<'_, Engine>, track: i64, pos: Option<usize>) -> Result<(), String> {
    let pos = pos.unwrap_or(0);
    // Window first, plugin second — same order and same reason as `vst_unload`.
    plugins::forget(fx_editor_slot(track, pos));
    plugins::drop_patch(&format!("f{track}.{pos}"));
    let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(Cmd::FxVstUnload { track, pos });
    }
    Ok(())
}

#[tauri::command(async)]
pub fn fxvst_editor_open(track: i64, pos: Option<usize>) -> Result<(), String> {
    plugins::editor_open(fx_editor_slot(track, pos.unwrap_or(0)))
}

#[tauri::command(async)]
pub fn fxvst_editor_close(track: i64, pos: Option<usize>) -> Result<(), String> {
    plugins::editor_close(fx_editor_slot(track, pos.unwrap_or(0)));
    Ok(())
}

// ---- plugin parameter automation ---------------------------------------------------------------

/// The automatable parameters of the plugin in an instrument slot — for the automation picker.
#[tauri::command(async)]
pub fn vst_params(slot: usize) -> Vec<ParamInfo> {
    recall_params(&format!("i{slot}"))
}

/// The same for a channel effect (`track` -1 = master), by its place in the chain.
#[tauri::command(async)]
pub fn fxvst_params(track: i64, pos: Option<usize>) -> Vec<ParamInfo> {
    recall_params(&format!("f{track}.{}", pos.unwrap_or(0)))
}

/// One normalised parameter change for an instrument plugin. Fired by automation lanes at up to
/// ~30 Hz per lane; fire-and-forget on the UI side, a message to the audio thread here.
#[tauri::command(async)]
pub fn vst_param_set(
    state: tauri::State<'_, Engine>,
    slot: usize,
    id: u32,
    value: f64,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(Cmd::VstParam { slot, id, value });
    }
    Ok(())
}

/// The same for a channel/master effect plugin, by its place in the chain.
#[tauri::command(async)]
pub fn fxvst_param_set(
    state: tauri::State<'_, Engine>,
    track: i64,
    pos: Option<usize>,
    id: u32,
    value: f64,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|_| "engine lock poisoned")?;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(Cmd::FxVstParam { track, pos: pos.unwrap_or(0), id, value });
    }
    Ok(())
}
