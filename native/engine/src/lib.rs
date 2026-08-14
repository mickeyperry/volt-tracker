//! VOLT's audio engine, in Rust.
//!
//! The rule for this crate: **it is a translation, not a rewrite.** Every instrument here has a
//! twin in `beta2.html`'s `trig()`, and the Rust is kept in the same order, with the same node
//! types and the same numbers, so the two can be read side by side. Where the JS says
//! `g.gain.setTargetAtTime(0, t + 0.004, p.decay / 3.2)`, so does this.
//!
//! That discipline is what makes `tests/parity.test.js` meaningful: both engines render the same
//! note, and the difference between them is measured in dB. A port is "done" when that number is
//! at the noise floor — not when it sounds about right.

use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::PI;
use web_audio_api::context::BaseAudioContext;
use web_audio_api::{PeriodicWave, PeriodicWaveOptions};
use web_audio_api::node::{
    AudioBufferSourceNode, AudioNode, AudioScheduledSourceNode, BiquadFilterNode, BiquadFilterType,
    ConstantSourceNode, GainNode, OscillatorNode, OscillatorType, OverSampleType, WaveShaperNode,
};
use web_audio_api::AudioBuffer;

pub mod clock;
pub mod fx;
pub mod mixer;
pub mod vst;
pub mod wav;

/// One note to render, as the parity harness describes it. Mirrors `trig()`'s arguments.
#[derive(Debug, Deserialize)]
pub struct Job {
    pub instr: String,
    pub note: f32,
    pub vel: f32,
    #[serde(default)]
    pub params: HashMap<String, f32>,
    pub dur: f64,
    #[serde(default = "def_sr")]
    pub sr: f32,
    /// When the note is struck. The browser side uses the same offset so both renders line up
    /// sample for sample rather than needing to be cross-correlated.
    #[serde(default = "def_t")]
    pub t: f64,
    /// `Bxx` — play the sample backwards. The value part of Bxx trims the start, which arrives as
    /// the `off` parameter, so all this needs to carry is the flag.
    #[serde(default)]
    pub rev: bool,
    /// Mixer settings to route the note through, when the job is testing the channel strip rather
    /// than the instrument. Absent means the bare graph the instrument jobs use — voice straight to
    /// the destination, nothing in the way.
    #[serde(default)]
    pub strip: Option<crate::mixer::TrackSet>,
    /// The master bus settings, when the job is testing everything *after* the strip — the send
    /// buses, the master fader and the safety limiter. Requires `strip`, because in both engines
    /// the only way into the master is through a channel.
    #[serde(default)]
    pub mix: Option<MixJob>,
}

/// The master half of a strip job. Mirrors what `buildGraph()` sets up in the browser.
#[derive(Deserialize, Debug, Clone, Copy)]
pub struct MixJob {
    /// The master fader. The browser's default is 0.75.
    pub master: f32,
    /// Ping-pong delay time in seconds — `setDelayTime()`'s `60/bpm*0.75`.
    #[serde(rename = "delayTime")]
    pub delay_time: f32,
}
fn def_sr() -> f32 {
    44100.0
}
fn def_t() -> f64 {
    0.05
}

impl Job {
    /// Optional parameter with a default. The strict `p()` is right for anything an instrument
    /// cannot sound without; this is for the ones the browser itself treats as absent-means-zero
    /// (sample trim, pad, Sxx offset), where panicking would reject a perfectly ordinary note.
    fn opt(&self, k: &str, dflt: f32) -> f32 {
        *self.params.get(k).unwrap_or(&dflt)
    }

    /// Parameter lookup that fails loudly. A silently-defaulted parameter is the one bug this
    /// harness could never catch by listening, since both engines would still make *a* sound.
    fn p(&self, k: &str) -> f32 {
        *self
            .params
            .get(k)
            .unwrap_or_else(|| panic!("job is missing parameter `{k}` for instrument `{}`", self.instr))
    }
}

/// `seedRnd` from beta2.html — and the reason a render is the take you heard.
///
/// This must stay bit-identical to the JS or every noise-based instrument (hat, snare, clap, the
/// kick's transient) drifts apart immediately. Two traps, both measured, both worth a comment:
///
///   * **`s >> 17` in JavaScript is an ARITHMETIC shift.** `>>` coerces to int32 first, so once
///     the high bit is set the shift sign-extends and the result is nothing like the logical
///     `>>` a textbook xorshift32 uses. Getting this wrong makes noise that is still perfectly
///     good white noise — just not *the same* white noise, which is undetectable by ear and
///     obvious in a null test. (`<<` and `^` are safe: identical for signed and unsigned.)
///   * **Divide in f64, narrow afterwards.** The JS computes `s / 2^32` at double precision and
///     only loses bits when it lands in the Float32Array. Doing it in f32 rounds too early.
pub fn seed_rnd(seed: u32) -> impl FnMut() -> f64 {
    let mut s: u32 = if seed == 0 { 1 } else { seed };
    move || {
        s ^= s << 13;
        s ^= ((s as i32) >> 17) as u32;
        s ^= s << 5;
        s as f64 / 4294967296.0
    }
}

/// The shared 2-second noise buffer every hat, snare, clap and glitch draws from.
/// `buildGraph()` in beta2.html, seeded with 0x5EED17.
pub fn noise_buffer<C: BaseAudioContext>(ctx: &C) -> AudioBuffer {
    let sr = ctx.sample_rate();
    let len = (sr as usize) * 2;
    let mut buf = ctx.create_buffer(1, len, sr);
    let mut rnd = seed_rnd(0x5EED17);
    let data: Vec<f32> = (0..len).map(|_| (rnd() * 2.0 - 1.0) as f32).collect();
    buf.copy_to_channel(&data, 0);
    buf
}

/// `driveCurveArr` — tanh saturation, normalised so the curve still reaches ±1.
pub fn drive_curve(amt: f32) -> Vec<f32> {
    let n = 512;
    let k = 1.0 + amt * 24.0;
    (0..n)
        .map(|i| {
            let x = i as f32 / (n - 1) as f32 * 2.0 - 1.0;
            (x * k).tanh() / k.tanh()
        })
        .collect()
}

/// Everything a voice needs from the surrounding engine. In the browser these are globals on the
/// AudioContext; here they are passed in, which is also what will let the live engine hold more
/// than one of them.
/// Generic over the context type, and that is the point rather than tidiness.
///
/// This started as `&OfflineAudioContext`, because the only caller was the parity harness. The
/// cost showed up the moment anything had to make a sound in real time: `live.rs` could not call
/// `trig()` at all and had to keep its own hand-copied kick and hat, so the thing being played was
/// provably not the thing being null-tested. Two implementations of one instrument is exactly the
/// drift this whole port exists to avoid.
///
/// `BaseAudioContext` is implemented by both `OfflineAudioContext` and `AudioContext`, so one
/// `trig()` now serves the harness and the live engine — same nodes, same automation, same code
/// path, whether it is being measured or heard.
pub struct Ctx<'a, C: BaseAudioContext> {
    pub ctx: &'a C,
    pub noise: &'a AudioBuffer,
    /// Band-limited wavetables, built on demand and keyed by (waveform, octave).
    pub waves: RefCell<HashMap<(u8, i32), PeriodicWave>>,
    /// The decoded audio for a sampler voice, when there is one.
    ///
    /// Deliberately not part of `Job`: a note is a small value that crosses IPC, and a sample is
    /// megabytes that must be decoded once and shared. The caller loads it and hands it in.
    pub sample: Option<&'a AudioBuffer>,
    /// Reversed copies, for Bxx. See `reversed()`.
    rev: RefCell<HashMap<(usize, usize), AudioBuffer>>,
    /// Drive-curve tables by amount×1000 — see `shaper()`. Built once per distinct drive setting
    /// instead of once per note.
    curves: RefCell<HashMap<i32, Vec<f32>>>,
}

impl<'a, C: BaseAudioContext> Ctx<'a, C> {
    pub fn new(ctx: &'a C, noise: &'a AudioBuffer) -> Self {
        Self { ctx, noise, waves: RefCell::new(HashMap::new()), sample: None,
               rev: RefCell::new(HashMap::new()), curves: RefCell::new(HashMap::new()) }
    }

    /// Same context, with a sample attached — for a `sampler` voice.
    pub fn with_sample(mut self, buf: &'a AudioBuffer) -> Self {
        self.sample = Some(buf);
        self
    }

    /// A reversed copy of `buf`, built once and kept.
    ///
    /// `revBuf` in beta2.html does the same thing for the same reason: a Bxx on every row of a
    /// pattern would otherwise reverse the whole sample sixteen times a bar. The cache is keyed by
    /// length and channel count rather than by identity, which is enough here because a `Ctx`
    /// holds exactly one sample.
    fn reversed(&self, buf: &AudioBuffer) -> AudioBuffer {
        let key = (buf.length(), buf.number_of_channels());
        if let Some(b) = self.rev.borrow().get(&key) {
            return b.clone();
        }
        let mut out = self.ctx.create_buffer(key.1, key.0, buf.sample_rate());
        for ch in 0..key.1 {
            let mut d = buf.get_channel_data(ch).to_vec();
            d.reverse();
            out.copy_to_channel(&d, ch);
        }
        self.rev.borrow_mut().insert(key, out.clone());
        out
    }
}

/// The Fourier series the Web Audio spec defines for the built-in waveforms, truncated to the
/// harmonics that fit under Nyquist at `f0`. Index 0 is DC and always zero.
///
/// This is the whole band-limiting story. The crate's own `sawtooth` and `square` are naive — at
/// 1760 Hz they differ from Chrome by -21 dB, which is audible fizz, and they run 1.18x hot
/// because Chrome normalises *after* band-limiting and the Gibbs overshoot eats the headroom.
/// Feeding the series through a normalised `PeriodicWave` fixes the shape and the level together,
/// because the normalisation falls out of the same arithmetic Chrome does.
fn harmonics(kind: OscillatorType, f0: f32, sample_rate: f32) -> Vec<f32> {
    let nyquist = sample_rate * 0.5;
    // At least one harmonic, and never more than the table can represent.
    let n = ((nyquist / f0.max(1.0)) as usize).clamp(1, 4096);
    let mut imag = vec![0.0f32; n + 1];
    for k in 1..=n {
        let kf = k as f32;
        imag[k] = match kind {
            OscillatorType::Sawtooth => {
                // (-1)^(k+1) * 2/(k pi)
                if k % 2 == 1 { 2.0 / (kf * PI) } else { -2.0 / (kf * PI) }
            }
            // 2/(k pi) * (1 - (-1)^k)  =>  4/(k pi) for odd k, 0 for even
            OscillatorType::Square => {
                if k % 2 == 1 { 4.0 / (kf * PI) } else { 0.0 }
            }
            // 8 sin(k pi / 2) / (pi^2 k^2)
            OscillatorType::Triangle => 8.0 * (kf * PI / 2.0).sin() / (PI * PI * kf * kf),
            _ => 0.0,
        };
    }
    imag
}

impl<'a, C: BaseAudioContext> Ctx<'a, C> {
    /// `osc(type, f)`
    ///
    /// Sine passes straight through — it has one harmonic and measures exact. The other three are
    /// rebuilt as band-limited periodic waves, cached per waveform per octave the way Chrome
    /// caches its wavetable ranges, so a song full of saw notes doesn't rebuild a table per note.
    fn osc(&self, kind: OscillatorType, f: f32) -> OscillatorNode {
        let mut o = self.ctx.create_oscillator();
        match kind {
            // Measured, not assumed. Tables are a large win for saw and square, whose built-ins
            // are naive: -15 dB against Chrome becomes -36 dB and -44 dB. For TRIANGLE the built-in
            // is better at both ends of the keyboard (-66 dB vs -47 at 110 Hz, -38 vs -28 at
            // 1760 Hz) — its harmonics fall away as 1/k^2, so it barely aliases, and routing it
            // through an 8192-point interpolated table only adds error. So: tables where they
            // help, built-in where it doesn't. Re-measure before changing this.
            OscillatorType::Sawtooth | OscillatorType::Square => {
                o.set_periodic_wave(self.wave(kind, f));
            }
            _ => o.set_type(kind),
        }
        o.frequency().set_value(f);
        o
    }

    /// Band-limited table cache, one table per waveform per OCTAVE, built for the top of the
    /// octave so nothing inside it can alias.
    ///
    /// Octave buckets are not an approximation chosen for speed — they are what Chrome does, and
    /// the measurements say so. Keying on the exact Nyquist harmonic count instead sounds more
    /// correct and matches Chrome *worse* everywhere: square at 1760 Hz goes from -44 dB to
    /// -21 dB, because Chrome uses five odd harmonics there where exact Nyquist allows six.
    /// Chrome additionally crossfades between neighbouring octave tables, which is most of the
    /// gap still showing on `probe-saw-hi`. Don't "fix" this without re-running the harness.
    fn wave(&self, kind: OscillatorType, f: f32) -> PeriodicWave {
        let band = f.max(1.0).log2().floor() as i32;
        let key = (if kind == OscillatorType::Sawtooth { 0u8 } else { 1 }, band);
        if let Some(w) = self.waves.borrow().get(&key) {
            return w.clone();
        }
        let top = 2f32.powi(band + 1); // the worst case inside this octave
        let imag = harmonics(kind, top, self.ctx.sample_rate());
        let w = self.ctx.create_periodic_wave(PeriodicWaveOptions {
            real: Some(vec![0.0; imag.len()]),
            imag: Some(imag),
            disable_normalization: false,
        });
        self.waves.borrow_mut().insert(key, w.clone());
        w
    }
    /// `mkNoise(t, dur)` — a looping window onto the shared buffer, not a fresh one per hit.
    fn noise(&self, t: f64, dur: f64) -> AudioBufferSourceNode {
        let mut s = self.ctx.create_buffer_source();
        s.set_buffer(self.noise.clone());
        s.set_loop(true);
        s.start_at(t);
        s.stop_at(t + dur);
        s
    }
    /// `mkShaper(amt)`
    /// `mkShaper(amt)` — with the CURVE CACHED, which is an RT-hygiene fix and not a micro-
    /// optimisation. Every kick, psybass, acid303, reese and mutant note built a fresh 512-point
    /// table (mutant: 2048) on the audio thread and shipped it to the render thread mid-note.
    /// A drum pattern at 174 bpm does that dozens of times a second, and the peak-load line in
    /// volt.log is what it costs: blocks over 100% of budget on note-dense bars while the average
    /// sat at 6-8% — a spike profile, i.e. crackle that arrives WITH THE NOTES. Quantised to
    /// 1/1000 because the curve is smooth in `amt` and a knob cannot hear that step.
    fn shaper(&self, amt: f32) -> WaveShaperNode {
        let mut ws = self.ctx.create_wave_shaper();
        let key = (amt.clamp(0.0, 8.0) * 1000.0).round() as i32;
        let curve = self
            .curves
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| drive_curve(key as f32 / 1000.0))
            .clone();
        ws.set_curve(curve);
        ws.set_oversample(OverSampleType::X2);
        ws
    }
    fn biquad(&self, kind: BiquadFilterType, freq: f32) -> BiquadFilterNode {
        let mut f = self.ctx.create_biquad_filter();
        f.set_type(kind);
        f.frequency().set_value(freq);
        f
    }
    fn gain(&self) -> GainNode {
        self.ctx.create_gain()
    }
}

/// `nfreq(note)` — MIDI note to Hz, A4 = 440 at note 57.
pub fn nfreq(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 57.0) / 12.0)
}

/// A sounding note, and the two ways to end one — `baseVoice` from beta2.html:1654.
///
/// `release(t)` fades over the instrument's own release time; `cut(t)` does it in 2.5 ms, which is
/// what a new note does to the note before it on the same track. Both are one-shot: the browser's
/// `dead` flag is here for the same reason, so a cut followed by a release cannot re-open an
/// envelope that has already been taken down.
///
/// `baseVoice`'s bend and filter live here now too: `bend` is the browser's ConstantSource wired
/// into every pitched source's `detune`, which is what makes glide (Gxx) and the pitch slides
/// (Uxx/Dxx) one ramp instead of per-oscillator bookkeeping; `filter` is the handle Cxx sweeps.
/// They existed in the JS from the start and are ported the day the engine grew a caller: a
/// reese line is 70% commands, and an engine that dropped them played it "berserk".
pub struct Voice {
    /// The voice's output gain — connect it where the browser connects `g`.
    pub out: GainNode,
    /// `rel` from `trig()`: the instrument's release time, 0.05 s for anything without one.
    pub rel: f32,
    dead: bool,
    /// This voice was CUT (a new note took its lane), not released — so it is silent in
    /// milliseconds and may be dropped accordingly. See [`Voice::quiet_by`].
    cut_short: bool,
    /// The note this voice was born on — glide targets are measured from it, exactly as
    /// `v.note` anchors `glideTo` in the browser.
    note: f32,
    /// Accumulated bend in cents (`v.bendVal`).
    bend_val: f32,
    /// The browser's `bendSrc`: one ConstantSource into every pitched source's detune. `None` for
    /// instruments whose sources are one-shots that outrun any ramp (drums).
    bend: Option<ConstantSourceNode>,
    /// The Cxx handle — the voice's own filter, where the instrument has one (`v.filter`).
    filter: Option<BiquadFilterNode>,
    /// Nodes the graph must keep alive. The browser's graph owns them; here the voice does, and
    /// dropping the voice is what finally frees them. This used to be `mem::forget`, which was
    /// fine for a one-shot render and a slow leak in an engine that runs all evening.
    /// Never read, and that is correct: holding the nodes IS the job, and dropping them is the
    /// only thing that ever happens to them.
    #[allow(dead_code)]
    parts: Vec<Box<dyn std::any::Any>>,
    /// Sources with no stop of their own — an oscillator told to start and never told to stop.
    /// These MUST be stopped explicitly or they play forever: the render graph will not free a node
    /// that still claims a tail, whether or not anything holds a handle to it. Self-terminating
    /// sources (every noise window, every sample source) sit in `parts` instead, exactly as the
    /// browser leaves them to end themselves.
    sustained: Vec<Box<dyn FnMut(f64)>>,
}

impl Voice {
    /// Connect the voice's output onward. The browser connects `g` to the channel strip; the live
    /// engine has no mixer yet and connects straight to the destination.
    pub fn connect<N: AudioNode>(&self, dest: &N) {
        self.out.connect(dest);
    }
    /// True once `release` or `cut` has been applied. `ch.voice.dead` in beta2.html.
    pub fn dead(&self) -> bool {
        self.dead
    }
    /// beta2.html:1658 — note-OFF, and what a pattern's end does to a held note.
    pub fn release(&mut self, t: f64) {
        if self.dead {
            return;
        }
        self.dead = true;
        let r = (self.rel.max(0.005)) as f64;
        self.out.gain().cancel_scheduled_values(t);
        self.out.gain().set_target_at_time(0.0, t, r / 3.0);
        let end = t + r * 4.0 + 0.1;
        self.sustained.iter_mut().for_each(|s| s(end));
        if let Some(b) = &mut self.bend {
            b.stop_at(end); // `bend.stop(t+r*4+.1)` — the bend dies with the voice
        }
    }
    /// beta2.html:1661 — 2.5 ms, the retrigger. Fast enough not to overlap the next note, slow
    /// enough not to click, and those two constraints are the whole of the number.
    pub fn cut(&mut self, t: f64) {
        if self.dead {
            return;
        }
        self.dead = true;
        self.cut_short = true;
        self.out.gain().cancel_scheduled_values(t);
        self.out.gain().set_target_at_time(0.0, t, 0.0025);
        self.sustained.iter_mut().for_each(|s| s(t + 0.06));
        if let Some(b) = &mut self.bend {
            b.stop_at(t + 0.06);
        }
    }
    /// When this voice is certainly silent and can be dropped, given the moment it was ended.
    /// `setTargetAtTime` never reaches zero, so "silent" is a number of time constants: ten is
    /// -87 dB, which is below anything a converter can represent.
    ///
    /// **A CUT voice is silent in milliseconds, not in a release time — and conflating the two
    /// cost dense patterns their audio.** `cut()` uses a 2.5 ms time constant and stops every
    /// source at +60 ms (the browser's `s.stop(t+.06)`), yet this returned the RELEASE figure for
    /// both endings: 1.1 s for a reese. Every retriggered note therefore left its filters,
    /// waveshaper (2x oversampled, and never free) and gains rendering in the graph for a second
    /// after it was inaudible. One note a bar: harmless. Sixteenths at 174 bpm: about thirteen
    /// dead voices stacked on the live one, per channel, and the audio thread wearing all of it —
    /// which is why the distortion arrived with note DENSITY and vanished when the notes thinned
    /// out (Mickey, 2026-08-14: *"when im putting notes in 16/1 its distorts and when its like
    /// every bar - its not"*). The browser never had this: dropping its nodes is what its garbage
    /// collector does moments after they stop.
    pub fn quiet_by(&self, ended_at: f64) -> f64 {
        if self.cut_short {
            // silent by 25 ms (ten 2.5 ms time constants), every source stopped by 60 ms
            ended_at + 0.12
        } else {
            ended_at + (self.rel.max(0.005) as f64) * 10.0 + 0.2
        }
    }
    /// Uxx/Dxx — beta2.html:1773. Cents pile up (`v.bendVal += cents`), each slide ramping from
    /// wherever the last one landed.
    pub fn slide(&mut self, cents: f32, t: f64, dur: f64) {
        if let Some(b) = &self.bend {
            let p = b.offset();
            p.cancel_scheduled_values(t);
            p.set_value_at_time(self.bend_val, t);
            self.bend_val += cents;
            p.linear_ramp_to_value_at_time(self.bend_val, t + dur);
        }
    }
    /// Gxx — beta2.html:1775. The target is absolute — measured in cents from the note the voice
    /// was BORN on, not from wherever a slide left it — which is exactly what makes repeated
    /// glides land in tune.
    pub fn glide_to(&mut self, note: f32, t: f64, dur: f64) {
        if let Some(b) = &self.bend {
            let target = (note - self.note) * 100.0;
            let p = b.offset();
            p.cancel_scheduled_values(t);
            p.set_value_at_time(self.bend_val, t);
            p.linear_ramp_to_value_at_time(target, t + dur);
            self.bend_val = target;
        }
    }
    /// Whether Gxx can glide this voice, or must fall through to a fresh trigger — the browser's
    /// `ch.voice && !ch.voice.dead`, plus "does it even have a bend".
    pub fn can_glide(&self) -> bool {
        !self.dead && self.bend.is_some()
    }
    /// Cxx — beta2.html:1649. The browser's exact curve: cancel, then a 15 ms setTarget onto the
    /// voice's own filter. A voice without one (drums, sub) ignores it, as the browser does.
    pub fn set_filter_target(&self, hz: f32, t: f64) {
        if let Some(f) = &self.filter {
            let p = f.frequency();
            p.cancel_scheduled_values(t);
            p.set_target_at_time(hz, t, 0.015);
        }
    }
}

/// Wrap a source that has no stop of its own so the voice can stop it later. Takes ownership: the
/// closure is the only handle, which is what keeps the node alive until the voice is dropped.
fn sustained<S: AudioScheduledSourceNode + 'static>(mut s: S) -> Box<dyn FnMut(f64)> {
    Box::new(move |t| s.stop_at(t))
}

/// The ported half of `trig()`. Returns the voice, carrying every node it built; the caller
/// connects it onward exactly as the JS connects `g` to the channel strip.
pub fn trig<C: BaseAudioContext>(c: &Ctx<C>, job: &Job) -> Result<Voice, String> {
    let t = job.t;
    let vel = job.vel;
    let note = job.note;
    let g = c.gain();
    // Everything the voice owns. `keep!` replaces the old free `keep()`, which was `mem::forget`.
    let mut owned: Vec<Box<dyn std::any::Any>> = Vec::new();
    let mut holds: Vec<Box<dyn FnMut(f64)>> = Vec::new();
    macro_rules! keep {
        ($e:expr) => {
            owned.push(Box::new($e) as Box<dyn std::any::Any>)
        };
    }
    macro_rules! hold {
        ($e:expr) => {
            holds.push(sustained($e))
        };
    }
    // The bend and the Cxx handle, set by the sustained instruments below and carried out on the
    // Voice — `baseVoice`'s bendSrc and v.filter, one to one. `mk_bend` is called lazily so the
    // one-shot drums never pay for a ConstantSource they cannot use.
    let mut vbend: Option<ConstantSourceNode> = None;
    let mut vfilter: Option<BiquadFilterNode> = None;
    macro_rules! bend_into {
        ($src:expr) => {{
            if vbend.is_none() {
                let mut b = c.ctx.create_constant_source();
                b.offset().set_value(0.0);
                b.start_at(t);
                vbend = Some(b);
            }
            vbend.as_ref().unwrap().connect($src.detune());
        }};
    }
    // beta2.html:1699 — these instruments release over their own `rel`; everything else uses 50 ms.
    // The JS `|| .05` means a rel of exactly zero falls back too, and that is copied rather than
    // tidied: a zero here would make `release()` a cut.
    // `mut` because two instruments override it at the baseVoice call instead of the map:
    // psybass passes a hard .03 (beta2.html:1972) — the "no hanging release" that makes it roll.
    let mut rel = match job.instr.as_str() {
        "sub" | "reese" | "mutant" | "supersaw" | "sampler" | "acid303" | "sliced" | "glitch" => {
            let r = job.opt("rel", 0.0);
            if r > 0.0 {
                r
            } else {
                0.05
            }
        }
        _ => 0.05,
    };

    match job.instr.as_str() {
        // ---- kick ----------------------------------------------------------------------------
        // beta2.html:1702. Sine with a fast exponential pitch drop, plus a high-passed noise tick
        // that is what actually reads as "punch" on a small speaker.
        "kick" => {
            let (pitch, punch, decay, drive) =
                (job.p("pitch"), job.p("punch"), job.p("decay"), job.p("drive"));
            let f0 = pitch * 2f32.powf((note - 48.0) / 12.0);
            let mut o = c.osc(OscillatorType::Sine, f0);
            o.frequency().set_value_at_time(f0.max(f0 * (3.5 + 4.0 * punch)), t);
            o.frequency()
                .exponential_ramp_to_value_at_time(f0, t + 0.03 + 0.035 * punch as f64);

            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel, t + 0.002);
            g.gain().set_target_at_time(0.0, t + 0.004, (decay / 3.2) as f64);

            if drive > 0.0 {
                let sh = c.shaper(drive);
                o.connect(&sh);
                sh.connect(&g);
                std::mem::forget(sh);
            } else {
                o.connect(&g);
            }

            let ns = c.noise(t, 0.012);
            let nf = c.biquad(BiquadFilterType::Highpass, 3500.0);
            let ng = c.gain();
            ng.gain().set_value_at_time(vel * punch * 0.6, t);
            ng.gain().set_target_at_time(0.0, t + 0.001, 0.004);
            ns.connect(&nf);
            nf.connect(&ng);
            ng.connect(&g);

            o.start_at(t);
            o.stop_at(t + (decay * 4.0) as f64 + 0.15);
            keep!((o, ns, nf, ng));
        }

        // ---- snare ---------------------------------------------------------------------------
        // beta2.html:1715. Body triangle plus band-passed noise. Note `g.gain.value = 1` after the
        // envelopes — the snare shapes its two halves separately, not the sum.
        "snare" => {
            let (tone, snap, decay) = (job.p("tone"), job.p("snap"), job.p("decay"));
            let mut o = c.osc(OscillatorType::Triangle, tone);
            o.frequency().set_value_at_time(tone * 1.7, t);
            o.frequency().exponential_ramp_to_value_at_time(tone, t + 0.025);
            let bg = c.gain();
            bg.gain().set_value_at_time(vel * 0.55, t);
            bg.gain().set_target_at_time(0.0, t + 0.002, 0.025);
            o.connect(&bg);
            bg.connect(&g);

            let ns = c.noise(t, (decay * 4.0) as f64 + 0.1);
            let bp = c.biquad(BiquadFilterType::Bandpass, 1700.0);
            bp.q().set_value(0.8);
            let hp = c.biquad(BiquadFilterType::Highpass, 450.0);
            let ng = c.gain();
            ng.gain().set_value_at_time(vel * snap, t);
            ng.gain().set_target_at_time(0.0, t + 0.003, (decay / 3.0) as f64);
            ns.connect(&hp);
            hp.connect(&bp);
            bp.connect(&ng);
            ng.connect(&g);

            g.gain().set_value(1.0);
            o.start_at(t);
            o.stop_at(t + 0.15);
            keep!((o, bg, ns, bp, hp, ng));
        }

        // ---- hat -----------------------------------------------------------------------------
        // beta2.html:1727. The whole instrument is one high-pass and one exponential decay.
        "hat" => {
            let (tone, decay) = (job.p("tone"), job.p("decay"));
            let ns = c.noise(t, (decay * 4.0) as f64 + 0.1);
            let hp = c.biquad(BiquadFilterType::Highpass, tone);
            g.gain().set_value_at_time(vel * 0.5, t);
            g.gain().set_target_at_time(0.0, t + 0.001, (decay / 3.0) as f64);
            ns.connect(&hp);
            hp.connect(&g);
            keep!((ns, hp));
        }

        // ---- clap ----------------------------------------------------------------------------
        // beta2.html:1733. Three noise bursts 11 ms apart then a tail — the spacing is the clap.
        "clap" => {
            let (tone, decay) = (job.p("tone"), job.p("decay"));
            let bp = c.biquad(BiquadFilterType::Bandpass, tone);
            bp.q().set_value(1.4);
            bp.connect(&g);
            g.gain().set_value(1.0);

            let mut parts: Vec<Box<dyn std::any::Any>> = Vec::new();
            for k in 0..3 {
                let s = c.noise(t + k as f64 * 0.011, 0.011);
                let bg = c.gain();
                bg.gain().set_value(vel * 0.7);
                s.connect(&bg);
                bg.connect(&bp);
                parts.push(Box::new((s, bg)));
            }
            let tail = c.noise(t + 0.032, (decay * 4.0) as f64 + 0.1);
            let tg = c.gain();
            tg.gain().set_value_at_time(vel * 0.6, t + 0.032);
            tg.gain().set_target_at_time(0.0, t + 0.035, (decay / 3.0) as f64);
            tail.connect(&tg);
            tg.connect(&bp);
            keep!((bp, tail, tg, parts));
        }

        // ---- sub -----------------------------------------------------------------------------
        // beta2.html:1742. Sine plus an octave, and a 6 ms fade-in so it doesn't click.
        "sub" => {
            let harm = job.p("harm");
            let f = nfreq(note);
            let mut o = c.osc(OscillatorType::Sine, f);
            let mut o2 = c.osc(OscillatorType::Sine, f * 2.0);
            let g2 = c.gain();
            g2.gain().set_value(harm * 0.4);
            o.connect(&g);
            o2.connect(&g2);
            g2.connect(&g);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel, t + 0.006);
            o.start_at(t);
            o2.start_at(t);
            // Started and never stopped — `sub` holds until something ends the note, which is the
            // whole point of it. `srcs=[o,o2]` in the JS, and the same two nodes here; both take
            // the bend so Gxx/Uxx reach them, as `baseVoice` wires every src with a detune.
            bend_into!(o);
            bend_into!(o2);
            keep!(g2);
            hold!(o);
            hold!(o2);
        }

        // ---- reese -------------------------------------------------------------------------
        // beta2.html:1890. The dnb bass — and the instrument whose absence made "VST effects on
        // the Reese channel" physically impossible (the engine had nothing to feed the chain).
        // nv detuned saws into a drive shaper into a resonant lowpass whose cutoff breathes on a
        // sine LFO, plus an optional clean sine underneath. Sustained like `sub`: every source
        // starts and nothing stops until the note ends.
        "reese" => {
            let (voices, detune, cutoff, res) =
                (job.p("voices"), job.p("detune"), job.p("cutoff"), job.p("res"));
            let (lfo_rate, lfo_amt, drive, sub) =
                (job.p("lfoRate"), job.p("lfoAmt"), job.p("drive"), job.p("sub"));
            let f = nfreq(note);
            let nv = voices.round().max(1.0) as usize;
            let pre = c.gain();
            pre.gain().set_value((1.0 / nv as f32) * (1.0 + drive * 8.0));
            let sh = c.shaper(0.3 + drive * 0.7);
            let lp = c.biquad(BiquadFilterType::Lowpass, cutoff);
            lp.q().set_value(res);
            pre.connect(&sh);
            sh.connect(&lp);
            lp.connect(&g);
            for k in 0..nv {
                // the JS spread: nv voices across ±detune cents, centre-out; one voice sits dead on
                let det = if nv > 1 {
                    detune * (k as f32 / (nv - 1) as f32 * 2.0 - 1.0)
                } else {
                    0.0
                };
                let mut o = c.osc(OscillatorType::Sawtooth, f);
                o.detune().set_value(det);
                o.connect(&pre);
                o.start_at(t);
                // the bend SUMS with the per-voice detune spread on the same AudioParam, so a
                // glide moves the whole stack and keeps its width — exactly the browser's wiring
                bend_into!(o);
                hold!(o);
            }
            if sub > 0.0 {
                let mut so = c.osc(OscillatorType::Sine, f);
                let sg = c.gain();
                sg.gain().set_value(sub * 0.55);
                so.connect(&sg);
                sg.connect(&g);
                so.start_at(t);
                bend_into!(so);
                keep!(sg);
                hold!(so);
            }
            let mut lfo = c.osc(OscillatorType::Sine, lfo_rate);
            let lg = c.gain();
            lg.gain().set_value(lfo_amt * cutoff * 0.8);
            lfo.connect(&lg);
            lg.connect(lp.frequency());
            lfo.start_at(t);
            // the browser wires the bend into EVERY src with a detune, the wobble LFO included —
            // a glide detunes the wobble rate by the same cents. Inaudible, and copied because a
            // null test hears everything.
            bend_into!(lfo);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel * 0.9, t + 0.005);
            // Cxx's handle — `v.filter = lp`. The node moves to the Voice's filter slot instead
            // of the anonymous parts bag so the command can reach it.
            vfilter = Some(lp);
            keep!((pre, sh, lg));
            hold!(lfo);
        }

        // ---- supersaw ------------------------------------------------------------------------
        // beta2.html:1932. Seven detuned saws fanned across the stereo field, high-passed so the
        // stack doesn't mud the low end, low-passed for tone, with a slow settle to 75% after the
        // attack. The trance pad. Sustained: every saw runs until the note ends.
        "supersaw" => {
            let (detune, spread, hp_f, cutoff, att) =
                (job.p("detune"), job.p("spread"), job.p("hp"), job.p("cutoff"), job.p("att"));
            let f = nfreq(note);
            const N: usize = 7;
            let mix = c.gain();
            mix.gain().set_value(0.24);
            let hp = c.biquad(BiquadFilterType::Highpass, hp_f);
            let lp = c.biquad(BiquadFilterType::Lowpass, cutoff);
            lp.q().set_value(0.5);
            mix.connect(&hp);
            hp.connect(&lp);
            lp.connect(&g);
            for k in 0..N {
                let mut o = c.osc(OscillatorType::Sawtooth, f);
                // the JS spread: ±detune cents across the stack, and the pan alternates sides
                // widening outward — (k%2?1:-1)*(k+1)/N, exactly
                o.detune().set_value(detune * (k as f32 / (N - 1) as f32 * 2.0 - 1.0));
                let pan = c.ctx.create_stereo_panner();
                pan.pan()
                    .set_value(spread * (if k % 2 == 1 { 1.0 } else { -1.0 }) * (k + 1) as f32 / N as f32);
                o.connect(&pan);
                pan.connect(&mix);
                o.start_at(t);
                bend_into!(o);
                hold!(o);
                keep!(pan);
            }
            // Two envelope events at DIFFERENT anchors, copied not tidied: the ramp lands at
            // max(2 ms, att) but the settle starts at att itself — for att < 2 ms the settle
            // begins inside the ramp, and that is what the browser plays.
            g.gain().set_value_at_time(0.0, t);
            g.gain()
                .linear_ramp_to_value_at_time(vel, t + (att as f64).max(0.002));
            g.gain().set_target_at_time(vel * 0.75, t + att as f64, 0.4);
            vfilter = Some(lp);
            keep!((mix, hp));
        }

        // ---- psybass -------------------------------------------------------------------------
        // beta2.html:1955. The israeli full-on / IM rolling bass: one saw through a drive shaper
        // into a resonant lowpass that snaps shut, gone by `decay` — no hanging release. The
        // whole instrument is that self-closing shape; a long tail would smear the roll.
        "psybass" => {
            let (cutoff, env_amt, res, decay, drive, sub) = (
                job.p("cutoff"),
                job.p("envAmt"),
                job.p("res"),
                job.p("decay"),
                job.p("drive"),
                job.p("sub"),
            );
            let f = nfreq(note);
            let mut o = c.osc(OscillatorType::Sawtooth, f);
            let sh = c.shaper(0.25 + drive * 0.75);
            let lp = c.biquad(BiquadFilterType::Lowpass, cutoff);
            lp.q().set_value(res);
            lp.frequency()
                .set_value_at_time((cutoff + env_amt * 3800.0).min(16000.0), t);
            lp.frequency()
                .set_target_at_time(cutoff, t + 0.002, (decay as f64 / 4.0).max(0.015));
            o.connect(&sh);
            sh.connect(&lp);
            lp.connect(&g);
            o.start_at(t);
            o.stop_at(t + (decay * 4.0) as f64 + 0.1);
            bend_into!(o);
            if sub > 0.0 {
                let mut so = c.osc(OscillatorType::Sine, f);
                let sg = c.gain();
                sg.gain().set_value(sub * 0.7);
                so.connect(&sg);
                sg.connect(&g);
                so.start_at(t);
                so.stop_at(t + (decay * 4.0) as f64 + 0.1);
                bend_into!(so);
                keep!((so, sg));
            }
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel, t + 0.002);
            g.gain().set_target_at_time(0.0, t + 0.004, (decay / 3.0) as f64);
            // baseVoice(g, srcs, note, .03) — the hard-coded release IS the sound; see `mut rel`
            rel = 0.03;
            vfilter = Some(lp);
            keep!((o, sh));
        }

        // ---- pluck ---------------------------------------------------------------------------
        // beta2.html:1944. Two saws ±6 cents through a resonant lowpass whose cutoff snaps from
        // cutoff+envAmt*9000 back down — the acid stab. One-shot: both saws stop themselves.
        "pluck" => {
            let (cutoff, env_amt, res, decay) =
                (job.p("cutoff"), job.p("envAmt"), job.p("res"), job.p("decay"));
            let f = nfreq(note);
            let lp = c.biquad(BiquadFilterType::Lowpass, cutoff);
            lp.q().set_value(res);
            lp.frequency()
                .set_value_at_time(cutoff + env_amt * 9000.0, t);
            lp.frequency()
                .set_target_at_time(cutoff, t + 0.002, decay as f64 / 4.0);
            lp.connect(&g);
            for dt in [-6.0f32, 6.0] {
                let mut o = c.osc(OscillatorType::Sawtooth, f);
                o.detune().set_value(dt);
                o.connect(&lp);
                o.start_at(t);
                o.stop_at(t + (decay * 5.0) as f64 + 0.2);
                bend_into!(o);
                keep!(o);
            }
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel * 0.8, t + 0.003);
            g.gain().set_target_at_time(0.0, t + 0.005, (decay / 2.5) as f64);
            vfilter = Some(lp);
        }

        // ---- acid303 -------------------------------------------------------------------------
        // beta2.html:1974. Single-osc 303: resonant sweep, drive AFTER the filter (psybass drives
        // before — that ordering is the difference between their tones), sustaining until note-off
        // so Gxx glides and Cxx cutoff rides work like a real acid line.
        "acid303" => {
            let (cutoff, env_amt, res, decay, drive, wave) = (
                job.p("cutoff"),
                job.p("envAmt"),
                job.p("res"),
                job.p("decay"),
                job.p("drive"),
                job.p("wave"),
            );
            let f = nfreq(note);
            let mut o = c.osc(
                if wave >= 0.5 { OscillatorType::Square } else { OscillatorType::Sawtooth },
                f,
            );
            let lp = c.biquad(BiquadFilterType::Lowpass, cutoff);
            lp.q().set_value(res);
            lp.frequency()
                .set_value_at_time((cutoff + env_amt * 4200.0).min(16000.0), t);
            lp.frequency()
                .set_target_at_time(cutoff, t + 0.002, (decay as f64 / 3.0).max(0.02));
            let sh = c.shaper(0.2 + drive * 0.8);
            o.connect(&lp);
            lp.connect(&sh);
            sh.connect(&g);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel * 0.9, t + 0.003);
            o.start_at(t);
            bend_into!(o);
            hold!(o);
            vfilter = Some(lp);
            keep!(sh);
        }

        // ---- mutant --------------------------------------------------------------------------
        // beta2.html:1908. FM'd saw+square through a wavefolder into a screaming resonant LP,
        // with a shape-selectable LFO riding the cutoff — neuro growls to psy screeches.
        // Sustained: everything runs until the note ends. The FM index (`mg`) feeds the
        // oscillators' FREQUENCY, not detune — Hz, not cents, and the growl depends on it.
        "mutant" => {
            let (fm, ratio, fold, sub, cutoff, res) = (
                job.p("fm"),
                job.p("ratio"),
                job.p("fold"),
                job.p("sub"),
                job.p("cutoff"),
                job.p("res"),
            );
            let (lfo_rate, lfo_amt, lfo_shape) =
                (job.p("lfoRate"), job.p("lfoAmt"), job.p("lfoShape"));
            let f = nfreq(note);
            let mut o1 = c.osc(OscillatorType::Sawtooth, f);
            let mut o2 = c.osc(OscillatorType::Square, f);
            o2.detune().set_value(9.0);
            let pre = c.gain();
            pre.gain().set_value(0.38);
            let mut mo = c.osc(OscillatorType::Sine, f * ratio);
            let mg = c.gain();
            mg.gain().set_value(fm * f * 6.0);
            mo.connect(&mg);
            mg.connect(o1.frequency());
            mg.connect(o2.frequency());
            // The wavefolder — a custom curve, not `drive_curve`: a sine fold blended with a
            // touch of tanh, the blend riding `fold`. 2048 points, exactly the JS table.
            let mut ws = c.ctx.create_wave_shaper();
            ws.set_oversample(OverSampleType::X2);
            let kk = 1.0 + fold * 9.0;
            ws.set_curve(
                (0..2048)
                    .map(|i| {
                        let x = i as f32 / 2047.0 * 2.0 - 1.0;
                        (x * kk * PI / 2.0).sin() * (1.0 - fold * 0.25) + (x * 3.0).tanh() * fold * 0.25
                    })
                    .collect(),
            );
            let lp = c.biquad(BiquadFilterType::Lowpass, cutoff);
            lp.q().set_value(res);
            pre.connect(&ws);
            ws.connect(&lp);
            lp.connect(&g);
            o1.connect(&pre);
            o2.connect(&pre);
            o1.start_at(t);
            o2.start_at(t);
            mo.start_at(t);
            bend_into!(o1);
            bend_into!(o2);
            bend_into!(mo);
            hold!(o1);
            hold!(o2);
            hold!(mo);
            if sub > 0.0 {
                let mut so = c.osc(OscillatorType::Sine, f / 2.0);
                let sg = c.gain();
                sg.gain().set_value(sub * 0.6);
                so.connect(&sg);
                sg.connect(&g);
                so.start_at(t);
                bend_into!(so);
                keep!(sg);
                hold!(so);
            }
            let mut lfo = c.osc(
                if lfo_shape >= 1.5 {
                    OscillatorType::Square
                } else if lfo_shape >= 0.5 {
                    OscillatorType::Triangle
                } else {
                    OscillatorType::Sine
                },
                lfo_rate,
            );
            let lg = c.gain();
            lg.gain().set_value(lfo_amt * cutoff * 0.9);
            lfo.connect(&lg);
            lg.connect(lp.frequency());
            lfo.start_at(t);
            bend_into!(lfo);
            hold!(lfo);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel * 0.9, t + 0.004);
            vfilter = Some(lp);
            keep!((pre, mg, ws, lg));
        }

        // ---- glitch --------------------------------------------------------------------------
        // beta2.html:2012. A seeded burst of grains — psy ticks, FM clangs, pulse zaps — sharing
        // ONE set of sources whose frequency is re-scheduled per grain (fresh oscillators per
        // grain cost 736 nodes inside a single Rxx row and stuttered the audio thread; see the
        // JS comment). Deterministic on purpose: the seed folds the note and velocity through
        // JS's ToInt32, so the same cell scatters the same way every play — and the same way in
        // both engines, which is what makes a null test of a "random" instrument possible at
        // all. The rnd() CALL ORDER inside the grain loop is load-bearing for exactly that
        // reason: jitter, level, decay, detune, per grain, in that order.
        "glitch" => {
            let (src, pitch, sweep, hits, gap0) =
                (job.p("src"), job.p("pitch"), job.p("sweep"), job.p("hits"), job.p("gap"));
            let (decay, fall, chaos, accel, crush_amt) = (
                job.p("decay"),
                job.p("fall"),
                job.p("chaos"),
                job.p("accel"),
                job.p("crush"),
            );
            let (cutoff, res) = (job.p("cutoff"), job.p("res"));
            let f0 = pitch * 2f32.powf((note - 48.0) / 12.0);
            // seedRnd(note*2654435761 ^ Math.round(vel*127)*40503) — both operands pass through
            // ToInt32 before the XOR (truncate, then wrap mod 2^32, then reinterpret signed).
            let to_i32 = |v: f64| ((v as i64) & 0xFFFF_FFFF) as u32 as i32;
            let seed = (to_i32(note as f64 * 2654435761.0)
                ^ to_i32((vel as f64 * 127.0).round() * 40503.0)) as u32;
            let mut rnd = seed_rnd(seed);
            let n = hits.round().max(1.0) as usize;
            let ch2 = chaos as f64;
            // past res 12 the filter becomes a bandpass — the narrow psy zap
            let bandy = res > 12.0;
            let lp = c.biquad(
                if bandy { BiquadFilterType::Bandpass } else { BiquadFilterType::Lowpass },
                cutoff,
            );
            lp.q().set_value(res.max(0.0001));
            // mkCrush — the bitcrusher staircase, or nothing at amt ≤ .01. Plain `round` is
            // safe against JS's round-half-up here: x*steps can never land on exactly .5
            // (odd numerator over 1023 — see the port notes), so the two roundings agree.
            let crush = if crush_amt > 0.01 {
                let steps = 2f32.powf(1.0 + (1.0 - crush_amt) * 6.0).round().max(2.0);
                let mut ws = c.ctx.create_wave_shaper();
                ws.set_curve(
                    (0..1024)
                        .map(|i| {
                            let x = i as f32 / 1023.0 * 2.0 - 1.0;
                            (x * steps).round() / steps
                        })
                        .collect(),
                );
                Some(ws)
            } else {
                None
            };
            match &crush {
                Some(cr) => {
                    lp.connect(cr);
                    cr.connect(&g);
                }
                None => {
                    lp.connect(&g);
                }
            }
            // makeup, so presets sit at roughly one loudness — the JS comment has the why
            let src_gain = if src < 0.5 {
                1.9
            } else if src < 1.5 {
                1.0
            } else if src < 2.5 {
                0.7
            } else {
                1.0
            };
            g.gain().set_value(
                src_gain
                    * (if bandy { 1.0 + (res - 12.0) * 0.34 } else { 1.0 })
                    * (1.0 + crush_amt * 0.25),
            );
            let gg = c.gain();
            gg.gain().set_value_at_time(0.0, t);
            gg.connect(&lp);
            // ONE set of sources for the whole burst — which set depends on `src`
            let mut noise_src = None;
            let (mut car, mut modo, mut md) = (None, None, None);
            let (mut o1, mut o2) = (None, None);
            if src < 0.5 {
                let mut s = c.ctx.create_buffer_source();
                s.set_buffer(c.noise.clone());
                s.set_loop(true);
                let bp = c.biquad(BiquadFilterType::Bandpass, 350.0);
                bp.q().set_value(1.0 + res * 0.3);
                s.connect(&bp);
                bp.connect(&gg);
                noise_src = Some((s, bp));
            } else if src < 1.5 {
                let cr = c.osc(OscillatorType::Sine, f0);
                let mo = c.osc(OscillatorType::Sine, f0 * 1.41);
                let m = c.gain();
                m.gain().set_value(0.0);
                mo.connect(&m);
                m.connect(cr.frequency());
                cr.connect(&gg);
                car = Some(cr);
                modo = Some(mo);
                md = Some(m);
            } else if src < 2.5 {
                let a = c.osc(OscillatorType::Square, f0);
                let b = c.osc(OscillatorType::Sawtooth, f0 * 1.005);
                let og = c.gain();
                og.gain().set_value(0.5);
                a.connect(&gg);
                b.connect(&og);
                og.connect(&gg);
                o1 = Some(a);
                o2 = Some((b, og));
            } else {
                let a = c.osc(OscillatorType::Sine, f0);
                a.connect(&gg);
                o1 = Some(a);
            }
            let clampf = |v: f64, lo: f64, hi: f64| v.max(lo).min(hi);
            let (mut ti, mut gap) = (t, gap0 as f64);
            let (mut total, mut first): (f64, Option<f64>) = (0.0, None);
            let ratio = 2f64.powf(-accel as f64 * 0.9);
            for k in 0..n {
                let jitter_t = ch2 * (rnd() - 0.5) * gap * 0.9;
                let at = t.max(ti + jitter_t);
                if first.is_none() {
                    first = Some(at);
                }
                let kk = k as f64 / 1f64.max((n - 1) as f64);
                let lvl = vel as f64 * (1.0 - fall as f64 * kk) * (1.0 - ch2 * 0.55 * rnd());
                let dec = (decay as f64 * (1.0 - fall as f64 * 0.5 * kk) * (1.0 + ch2 * (rnd() - 0.5)))
                    .max(0.004);
                let detune = 1.0 + ch2 * (rnd() - 0.5) * 1.2;
                let fq = clampf(
                    f0 as f64 * detune * (1.0 + k as f64 * 0.03 * accel as f64),
                    20.0,
                    18000.0,
                );
                gg.gain().set_value_at_time(0.0, at);
                gg.gain().linear_ramp_to_value_at_time(lvl as f32, at + 0.0008);
                gg.gain().set_target_at_time(0.0, at + 0.0012, dec / 3.2);
                if let Some((_, bp)) = &noise_src {
                    // 0 — noise burst, the classic tick
                    bp.frequency().set_value_at_time(clampf(fq * 4.0, 40.0, 18000.0) as f32, at);
                    bp.frequency().exponential_ramp_to_value_at_time(
                        clampf(fq * 4.0 * 2f64.powf(sweep as f64 * 3.0), 40.0, 18000.0) as f32,
                        at + dec * 1.4,
                    );
                } else if let Some(cr) = &car {
                    // 1 — FM metal: clangs and bells
                    cr.frequency().set_value_at_time(fq as f32, at);
                    modo.as_ref().unwrap().frequency().set_value_at_time(
                        clampf(fq * (1.41 + ch2 * rnd() * 3.0), 20.0, 18000.0) as f32,
                        at,
                    );
                    let m = md.as_ref().unwrap();
                    m.gain()
                        .set_value_at_time((fq * (3.0 + 6.0 * sweep.abs() as f64)) as f32, at);
                    m.gain().set_target_at_time((fq * 0.4) as f32, at + 0.001, dec / 2.5);
                } else if let Some((b, _)) = &o2 {
                    // 2 — pulse: buzzy, vocal, rude
                    let a = o1.as_ref().unwrap();
                    a.frequency().set_value_at_time(fq as f32, at);
                    a.frequency().exponential_ramp_to_value_at_time(
                        clampf(fq * 2f64.powf(sweep as f64 * 4.0), 20.0, 18000.0) as f32,
                        at + dec * 1.6,
                    );
                    b.frequency().set_value_at_time((fq * 1.005) as f32, at);
                } else if let Some(a) = &o1 {
                    // 3 — sine zap: psy blips and lasers
                    a.frequency().set_value_at_time(
                        clampf(fq * 2f64.powf(-sweep as f64 * 2.5), 20.0, 18000.0) as f32,
                        at,
                    );
                    a.frequency().exponential_ramp_to_value_at_time(
                        clampf(fq * 2f64.powf(sweep as f64 * 2.5), 20.0, 18000.0) as f32,
                        at + dec * 1.8,
                    );
                }
                total = total.max(at - t + dec * 4.0 + 0.02);
                ti += gap;
                gap = clampf(gap * ratio, 0.002, 0.5);
            }
            // every source starts on the FIRST grain and dies together after the last tail
            let st = first.unwrap_or(t);
            let kill = t + total + 0.05;
            if let Some((s, bp)) = noise_src {
                let mut s = s;
                s.start_at(st);
                s.stop_at(kill);
                // a buffer source has a detune, and baseVoice wires the bend into everything
                // that has one — the noise tick tape-bends under Uxx like all the rest
                bend_into!(s);
                keep!((s, bp));
            }
            if let Some(mut cr) = car {
                cr.start_at(st);
                cr.stop_at(kill);
                bend_into!(cr);
                keep!(cr);
            }
            if let Some(mut mo) = modo {
                mo.start_at(st);
                mo.stop_at(kill);
                bend_into!(mo);
                keep!(mo);
            }
            if let Some(m) = md {
                keep!(m);
            }
            if let Some(mut a) = o1 {
                a.start_at(st);
                a.stop_at(kill);
                bend_into!(a);
                keep!(a);
            }
            if let Some((mut b, og)) = o2 {
                b.start_at(st);
                b.stop_at(kill);
                bend_into!(b);
                keep!((b, og));
            }
            vfilter = Some(lp);
            keep!((gg, crush));
        }

        // ---- sampler -----------------------------------------------------------------------------
        // beta2.html:1844. The instrument Mickey actually leans on, and the first non-synth here.
        //
        // The sample itself is not in `Job` — a note cannot carry a megabyte of audio. The caller
        // puts the decoded buffer in `Ctx::sample` and this plays it, which is also how `sliced`
        // will work when it lands. Without one loaded the JS returns null; so does this.
        //
        // Deliberately faithful to two details that look like rounding and are not:
        //   * the attack floor is 0.4 ms, not zero. A ramp to an exact zero-length target is a
        //     click in one engine and a discontinuity in the other; the browser picks 0.0004 so
        //     kicks keep their transient, and matching it is the difference between a null test
        //     at -90 dB and one at -30.
        //   * `dur` is measured in the buffer's own seconds, but the fade-out is scheduled in
        //     *played* seconds — hence `dur / rt`. Getting that wrong only shows up on notes
        //     transposed away from the root, which is exactly where nobody looks.
        "sampler" => {
            let Some(buf) = c.sample else {
                return Err("sampler has no sample loaded".into());
            };
            let (root, fin, fout, amp) = (job.p("root"), job.p("fin"), job.p("fout"), job.p("amp"));
            let rt = 2f32.powf((note - root) / 12.0);

            let mut s = c.ctx.create_buffer_source();
            // Bxx: play it backwards. The browser keeps a reversed copy cached per instrument
            // (`revBuf`) because reversing megabytes on every note would be absurd; here the cache
            // lives on `Ctx` for the same reason and with the same lifetime as the sample.
            s.set_buffer(if job.rev { c.reversed(buf) } else { buf.clone() });
            s.playback_rate().set_value(rt);

            // Trim region as fractions of the buffer. `s0`/`s1` and the Sxx offset default to the
            // whole sample.
            let (mut s0, mut s1) = (job.opt("s0", 0.0), job.opt("s1", 1.0));
            if job.rev {
                // The region has to flip with the audio, or reversing a trimmed sample plays the
                // wrong part of it — the mirrored span, not the same span backwards.
                let (a, b) = (1.0 - s1, 1.0 - s0);
                s0 = a;
                s1 = b;
            }
            let (s0, s1) = (s0, s1);
            let off = job.opt("off", 0.0);
            let bd = buf.duration();
            let st = (s0 + off * (s1 - s0)) as f64 * bd;
            let t0 = t + job.opt("pad", 0.0) as f64;
            let dur = (s1 as f64 * bd - st).max(0.001);

            let atk = fin.max(0.0004) as f64;
            let lv = vel * amp;
            g.gain().set_value_at_time(0.0, t0);
            g.gain().linear_ramp_to_value_at_time(lv, t0 + atk);
            if fout > 0.0 {
                let end = t0 + dur / rt as f64;
                let fs = (end - fout as f64).max(t0 + atk);
                g.gain().set_value_at_time(lv, fs);
                g.gain().linear_ramp_to_value_at_time(0.0, end);
            }

            s.connect(&g);
            s.start_at_with_offset_and_duration(t0, st, dur);
            // A buffer source has a detune too, and the browser's baseVoice wires the bend into
            // it — Uxx/Gxx on a sample is a tape-bend, and it worked in the webview all along.
            bend_into!(s);
            keep!(s);
        }

        // ---- sliced ------------------------------------------------------------------------------
        // beta2.html:1954. A chopped break: one buffer, a list of regions, and the note number
        // picks which region upward from `sliceBase`. This is how a loop gets played as an
        // instrument, so for a samples-only workflow it matters as much as the sampler does.
        //
        // The slice list arrives as flat `slice0s`/`slice0e` params rather than as a nested array,
        // because `Job::params` is a flat map of numbers on both sides of the IPC boundary and
        // keeping it that way is what lets the serde contract stay trivial. Ugly to read, and the
        // alternative is a second shape to keep in agreement.
        //
        // Note the attack: 0.4 ms flat, no `fin`. The browser does not offer a fade-in here and
        // adding one "for symmetry" would put a click-free ramp where a chop is supposed to be
        // abrupt — the transient IS the sound.
        "sliced" => {
            let Some(buf) = c.sample else {
                return Err("sliced has no sample loaded".into());
            };
            let n = job.opt("sliceCount", 0.0) as i32;
            if n <= 0 {
                return Err("sliced has no slices".into());
            }
            let base = job.opt("sliceBase", 48.0);
            let idx = ((note - base) as i32).clamp(0, n - 1);
            let (mut a0, mut a1) = (
                job.opt(&format!("slice{idx}s"), 0.0),
                job.opt(&format!("slice{idx}e"), 1.0),
            );

            let mut s = c.ctx.create_buffer_source();
            s.set_buffer(if job.rev { c.reversed(buf) } else { buf.clone() });
            s.playback_rate().set_value(job.opt("rate", 1.0).max(0.0001));
            if job.rev {
                let (a, b) = (1.0 - a1, 1.0 - a0);
                a0 = a;
                a1 = b;
            }

            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(vel, t + 0.0004);

            let bd = buf.duration();
            let st = (a0 + job.opt("off", 0.0) * (a1 - a0)) as f64 * bd;
            let dur = (a1 as f64 * bd - st).max(0.001);
            s.connect(&g);
            s.start_at_with_offset_and_duration(t, st, dur);
            keep!(s);
        }

        // ---- probes ----------------------------------------------------------------------------
        // Not instruments. When a null test fails, the failure is somewhere in a graph of five or
        // six nodes and "it sounds wrong" narrows nothing down. These are one-node graphs with an
        // exact twin in tests/parity.test.js, so a failure names the primitive that differs
        // instead of the instrument that contains it. Cheap to add, and the fastest way through a
        // port this size — leave them here.
        "probe-expramp" => {
            let mut o = c.osc(OscillatorType::Sine, 200.0);
            o.frequency().set_value_at_time(2000.0, t);
            o.frequency().exponential_ramp_to_value_at_time(200.0, t + 0.3);
            o.connect(&g);
            g.gain().set_value(0.5);
            o.start_at(t);
            o.stop_at(t + 0.9);
            keep!(o);
        }
        // Bare waveforms, low and high. Aliasing gets worse as the fundamental rises, so a
        // waveform that looks fine at 110 Hz can be a mess at 1760 Hz — measure both before
        // deciding the built-in oscillators need replacing.
        "probe-triangle" | "probe-saw" | "probe-square" | "probe-sine-hi" | "probe-triangle-hi"
        | "probe-saw-hi" | "probe-square-hi" => {
            let hi = job.instr.ends_with("-hi");
            let kind = if job.instr.contains("triangle") {
                OscillatorType::Triangle
            } else if job.instr.contains("saw") {
                OscillatorType::Sawtooth
            } else if job.instr.contains("square") {
                OscillatorType::Square
            } else {
                OscillatorType::Sine
            };
            // 190 Hz keeps the original triangle probe's number comparable across the whole log.
            let f = if hi { 1760.0 } else if kind == OscillatorType::Triangle { 190.0 } else { 110.0 };
            let mut o = c.osc(kind, f);
            o.connect(&g);
            g.gain().set_value(0.5);
            o.start_at(t);
            o.stop_at(t + 0.9);
            keep!(o);
        }
        // Is `exponential_ramp_to_value_at_time` itself wrong, or only wrong when it drives an
        // oscillator's frequency? Same ramp, on a gain.
        "probe-expgain" => {
            let mut o = c.osc(OscillatorType::Sine, 300.0);
            o.connect(&g);
            g.gain().set_value_at_time(1.0, t);
            g.gain().exponential_ramp_to_value_at_time(0.01, t + 0.3);
            o.start_at(t);
            o.stop_at(t + 0.9);
            keep!(o);
        }
        // And is a-rate frequency automation working at all? A linear ramp, same target param.
        "probe-linfreq" => {
            let mut o = c.osc(OscillatorType::Sine, 200.0);
            o.frequency().set_value_at_time(2000.0, t);
            o.frequency().linear_ramp_to_value_at_time(200.0, t + 0.3);
            o.connect(&g);
            g.gain().set_value(0.5);
            o.start_at(t);
            o.stop_at(t + 0.9);
            keep!(o);
        }
        // Clap starts its bursts 11 ms apart, which is not a whole number of samples. Does a
        // buffer source scheduled off the sample grid land in the same place in both engines?
        "probe-noiseoffset" => {
            let ns = c.noise(t + 0.011, 0.2);
            ns.connect(&g);
            g.gain().set_value(0.8);
            keep!(ns);
        }
        // The kick's noise tick decays with a 4 ms time constant — faster than a 2.9 ms render
        // quantum. If a gain's automation is computed per block rather than per sample, this is
        // where it shows and a slower envelope (the hat's 20 ms) would hide it.
        "probe-fasttarget" => {
            let ns = c.noise(t, 0.012);
            let ng = c.gain();
            ng.gain().set_value_at_time(0.48, t);
            ng.gain().set_target_at_time(0.0, t + 0.001, 0.004);
            ns.connect(&ng);
            ng.connect(&g);
            g.gain().set_value(1.0);
            keep!((ns, ng));
        }
        // The kick, cut in half. Its residue lives entirely in the first 12 ms, where both the
        // noise tick and the steepest part of the pitch sweep are — these two say which.
        // Numbers are kick-36-nodrive's: f0 = 25 Hz, punch 1, decay 0.8, vel 1.
        "probe-kicknoise" => {
            let ns = c.noise(t, 0.012);
            let nf = c.biquad(BiquadFilterType::Highpass, 3500.0);
            let ng = c.gain();
            ng.gain().set_value_at_time(0.6, t);
            ng.gain().set_target_at_time(0.0, t + 0.001, 0.004);
            ns.connect(&nf);
            nf.connect(&ng);
            ng.connect(&g);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(1.0, t + 0.002);
            g.gain().set_target_at_time(0.0, t + 0.004, 0.25);
            keep!((ns, nf, ng));
        }
        "probe-kicksine" => {
            let mut o = c.osc(OscillatorType::Sine, 25.0);
            o.frequency().set_value_at_time(187.5, t);
            o.frequency().exponential_ramp_to_value_at_time(25.0, t + 0.065);
            o.connect(&g);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(1.0, t + 0.002);
            g.gain().set_target_at_time(0.0, t + 0.004, 0.25);
            o.start_at(t);
            o.stop_at(t + 3.35);
            keep!(o);
        }
        // Both halves of the kick failed, and the only thing they share is this envelope:
        // a ramp up from zero, a 2 ms hold, then a decay. It is the shape under nearly every
        // instrument VOLT has, so if it differs, everything differs.
        "probe-kickenv" => {
            let mut o = c.osc(OscillatorType::Sine, 300.0);
            o.connect(&g);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(1.0, t + 0.002);
            g.gain().set_target_at_time(0.0, t + 0.004, 0.25);
            o.start_at(t);
            o.stop_at(t + 1.0);
            keep!(o);
        }
        // The same envelope with both events landing exactly on sample boundaries (88 and 256
        // samples in, 256 being a whole number of 128-sample quanta). If this nulls and
        // probe-kickenv does not, the difference is Chrome quantising automation to its render
        // blocks, not the port.
        "probe-kickenv-aligned" => {
            let sp = 1.0 / 44100.0;
            let mut o = c.osc(OscillatorType::Sine, 300.0);
            o.connect(&g);
            g.gain().set_value_at_time(0.0, t);
            g.gain().linear_ramp_to_value_at_time(1.0, t + 88.0 * sp);
            g.gain().set_target_at_time(0.0, t + 256.0 * sp, 0.25);
            o.start_at(t);
            o.stop_at(t + 1.0);
            keep!(o);
        }
        // The waveshaper, on its own. This one could be a REAL difference rather than an
        // automation artifact: 2x oversampling means an up/downsampling filter pair that the spec
        // does not pin down, and distortion products are audible in a way a 0.1 dB blip is not.
        "probe-shaper" => {
            let mut o = c.osc(OscillatorType::Sine, 110.0);
            let sh = c.shaper(0.3);
            let pre = c.gain();
            pre.gain().set_value(0.9);
            o.connect(&pre);
            pre.connect(&sh);
            sh.connect(&g);
            g.gain().set_value(0.7);
            o.start_at(t);
            o.stop_at(t + 2.0); // past the end of the render: this probe is about the shaper, not the stop
            keep!((o, sh, pre));
        }
        // ...and the stop on its own, because the two engines disagree by a sample there and it
        // was inflating every probe that ended inside its own render.
        "probe-stopedge" => {
            let mut o = c.osc(OscillatorType::Sine, 110.0);
            o.connect(&g);
            g.gain().set_value(0.7);
            o.start_at(t);
            o.stop_at(t + 1.0);
            keep!(o);
        }
        "probe-bandpass" => {
            let ns = c.noise(t, 0.5);
            let bp = c.biquad(BiquadFilterType::Bandpass, 1150.0);
            bp.q().set_value(1.4);
            ns.connect(&bp);
            bp.connect(&g);
            g.gain().set_value(0.9);
            keep!((ns, bp));
        }
        // The channel strip's compressor, on its own, at the settings a track with the compressor
        // knob at ZERO gets: ratio 1:1, so by every textbook it is a wire that costs 6 ms. Chrome's
        // is not a wire — its gain wanders between 0.93 and 1.00 following the envelope — and this
        // probe exists to hold that difference still and measure it, instead of letting it turn up
        // smeared across every mixer test. `params` carries the settings so the same arm can be
        // asked for the compressing case too.
        "probe-comp" => {
            let ns = c.noise(t, 0.5);
            let cmp = c.ctx.create_dynamics_compressor();
            cmp.set_channel_count_mode(web_audio_api::node::ChannelCountMode::Explicit);
            cmp.set_channel_count(2);
            cmp.threshold().set_value(job.opt("th", 0.0));
            cmp.knee().set_value(job.opt("knee", 6.0));
            cmp.ratio().set_value(job.opt("ratio", 1.0));
            cmp.attack().set_value(job.opt("att", 0.003));
            cmp.release().set_value(job.opt("relc", 0.12));
            ns.connect(&cmp);
            cmp.connect(&g);
            g.gain().set_value(job.opt("amp", 0.35));
            keep!((ns, cmp));
        }

        other => return Err(format!("instrument `{other}` is not ported yet")),
    }

    Ok(Voice {
        out: g,
        rel,
        dead: false,
        cut_short: false,
        note,
        bend_val: 0.0,
        bend: vbend,
        filter: vfilter,
        parts: owned,
        sustained: holds,
    })
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    use web_audio_api::context::OfflineAudioContext;

    const SR: f32 = 44100.0;

    /// Render one `sub` note, optionally ended part-way through, and return the buffer.
    ///
    /// A STEREO sample must come back out of the sampler bit-faithful, channels unswapped and
    /// unmangled — the frozen-channel path. Every sample before 2026-08-13 was mono, so this is
    /// the first time the stereo path is pinned ("extreme distortion... where the note is", on a
    /// frozen stereo bounce, was the report that demanded it).
    #[test]
    fn a_stereo_sample_plays_back_clean_and_unswapped() {
        let ctx = OfflineAudioContext::new(2, 48000, 48000.0);
        let noise = noise_buffer(&ctx);
        // L = 220 Hz at 0.9, R = 220 Hz at 0.45 — asymmetric so a swap or a mix-down shows.
        let mut smp = ctx.create_buffer(2, 24000, 48000.0);
        for ch in 0..2 {
            let amp = if ch == 0 { 0.9f32 } else { 0.45 };
            let d: Vec<f32> = (0..24000)
                .map(|i| amp * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48000.0).sin())
                .collect();
            smp.copy_to_channel(&d, ch);
        }
        let c = Ctx::new(&ctx, &noise).with_sample(&smp);
        let mut params = HashMap::new();
        for (k, v) in [("root", 48.0f32), ("rel", 0.04), ("amp", 1.0), ("fin", 0.0), ("fout", 0.0)] {
            params.insert(k.to_string(), v);
        }
        let job = Job {
            instr: "sampler".into(), note: 48.0, vel: 1.0, params, dur: 0.5, sr: 48000.0, t: 0.0,
            rev: false, strip: None, mix: None,
        };
        let v = trig(&c, &job).expect("sampler must trig");
        v.connect(&ctx.destination());
        let mut ctx = ctx;
        let out = ctx.start_rendering_sync();
        for ch in 0..2 {
            let src = smp.get_channel_data(ch);
            let o = out.get_channel_data(ch);
            // skip the 0.4 ms attack floor; compare the body
            let (a, b) = (200usize, 23000usize);
            let mut resid = 0.0f64;
            let mut power = 0.0f64;
            for i in a..b {
                let d = (o[i] - src[i]) as f64;
                resid += d * d;
                power += (src[i] as f64) * (src[i] as f64);
            }
            let db = 10.0 * (resid / power).log10();
            assert!(db < -60.0, "stereo sampler ch{ch} must be clean, residue {db:.1} dB");
        }
    }

    /// Gxx must GLIDE — continuous pitch, not a step — proven by ear-math: zero-crossing rate an
    /// octave up doubles, and the middle of the glide sits between the two. A step would jump.
    #[test]
    fn gxx_glides_a_held_sub_up_an_octave() {
        let ctx = OfflineAudioContext::new(1, (SR * 2.0) as usize, SR);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut params = HashMap::new();
        params.insert("harm".to_string(), 0.0f32); // fundamental only: crossings = frequency
        params.insert("rel".to_string(), 0.05f32);
        let job = Job {
            instr: "sub".into(), note: 36.0, vel: 1.0, params, dur: 0.5, sr: SR, t: 0.0,
            rev: false, strip: None, mix: None,
        };
        let mut v = trig(&c, &job).expect("sub must build");
        v.connect(&ctx.destination());
        v.glide_to(48.0, 0.8, 0.4); // one octave, 0.8 s in, over 0.4 s
        let mut ctx = ctx;
        let out = ctx.start_rendering_sync();
        let d = out.get_channel_data(0);
        let crossings = |from: f64, to: f64| -> usize {
            let (a, b) = ((from * SR as f64) as usize, (to * SR as f64) as usize);
            d[a..b].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
        };
        let before = crossings(0.3, 0.7) as f64; // held at 36
        let after = crossings(1.4, 1.8) as f64;  // settled at 48
        let ratio = after / before;
        assert!((ratio - 2.0).abs() < 0.15, "an octave glide must double the rate, got ×{ratio}");
        let mid = crossings(0.95, 1.05) as f64 / 0.1 * 0.4 / before; // rate mid-glide vs start
        assert!(mid > 1.15 && mid < 1.9, "mid-glide must sit BETWEEN the notes (got ×{mid}) — a step would not");
    }

    /// A held reese keeps sounding, releases over its own `rel`, and its LFO dies with it —
    /// the sustained-instrument contract `sub` established, on the newest port.
    #[test]
    fn reese_holds_and_releases_like_sub() {
        let params: std::collections::HashMap<String, f32> = [
            ("voices", 1.867f32), ("detune", 49.98), ("cutoff", 147.28), ("res", 0.0),
            ("lfoRate", 0.35), ("lfoAmt", 0.35), ("drive", 0.576), ("sub", 0.027), ("rel", 0.09),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        // Mickey's actual Reese from dnb5.volt — the fractional voice count (rounds to 2) and a
        // sub layer both exercised, because those are the paths a synthetic preset would skip.
        let render = |end: Option<f64>| -> Vec<f32> {
            let ctx = OfflineAudioContext::new(1, (SR * 2.0) as usize, SR);
            let noise = noise_buffer(&ctx);
            let c = Ctx::new(&ctx, &noise);
            let job = Job {
                instr: "reese".into(),
                note: 24.0,
                vel: 1.0,
                params: params.clone(),
                dur: 0.5,
                sr: SR,
                t: 0.0,
                rev: false,
                strip: None,
                mix: None,
            };
            let mut v = trig(&c, &job).expect("reese must trig");
            v.connect(&ctx.destination());
            if let Some(at) = end {
                v.release(at);
            }
            let mut ctx = ctx;
            let out = ctx.start_rendering_sync();
            out.get_channel_data(0).to_vec()
        };
        let held = render(None);
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak(&held[..(SR as usize / 2)]) > 0.01, "a reese must sound at all");
        assert!(
            peak(&held[(SR as usize * 3 / 2)..]) > 0.01,
            "and keep sounding while held — it is a sustained instrument"
        );
        let released = render(Some(1.0));
        assert!(
            peak(&released[((1.55 * SR) as usize)..]) < 1e-3,
            "released at 1.0 s with rel 0.09 it must be silent well before 1.55 s"
        );
    }

    /// `sub` is the instrument this matters for: its two oscillators are started and never given a
    /// stop, exactly as `srcs=[o,o2]` in beta2.html. Everything else in the engine terminates
    /// itself, so if release works here it works everywhere.
    fn render(end: Option<(&str, f64)>, rel: f32) -> Vec<f32> {
        let mut ctx = OfflineAudioContext::new(1, SR as usize, SR);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut params = HashMap::new();
        params.insert("harm".to_string(), 0.5f32);
        params.insert("rel".to_string(), rel);
        let job = Job {
            instr: "sub".into(),
            note: 36.0,
            vel: 1.0,
            params,
            dur: 0.5,
            sr: SR,
            t: 0.05,
            rev: false,
            strip: None,
            mix: None,
        };
        let mut v = trig(&c, &job).expect("sub should build");
        v.connect(&ctx.destination());
        match end {
            Some(("cut", t)) => v.cut(t),
            Some(("release", t)) => v.release(t),
            _ => {}
        }
        ctx.start_rendering_sync().get_channel_data(0).to_vec()
    }

    /// RMS over a window given in seconds.
    fn rms(buf: &[f32], from: f64, to: f64) -> f32 {
        let a = (from * SR as f64) as usize;
        let b = ((to * SR as f64) as usize).min(buf.len());
        let s: f32 = buf[a..b].iter().map(|x| x * x).sum();
        (s / (b - a) as f32).sqrt()
    }

    /// The control, and the bug this whole change is about: a `sub` with nothing done to it plays
    /// forever. If this ever stops sounding at the end of the render, the test below proves nothing.
    #[test]
    fn an_unended_sub_is_still_sounding_a_second_later() {
        let b = render(None, 0.05);
        assert!(rms(&b, 0.9, 1.0) > 0.3, "control note went quiet on its own");
    }

    #[test]
    fn cut_silences_a_held_note() {
        // Cut at 300 ms with the browser's 2.5 ms time constant: 100 ms later is forty time
        // constants, so anything above the float noise floor here means the sources kept running.
        let b = render(Some(("cut", 0.3)), 0.05);
        assert!(rms(&b, 0.15, 0.28) > 0.3, "note was not sounding before the cut");
        assert!(rms(&b, 0.4, 1.0) < 1e-4, "still sounding after cut: {}", rms(&b, 0.4, 1.0));
    }

    /// A retriggered lane must not leave a second of dead graph behind it.
    ///
    /// This is the density bug of 2026-08-14 in one assertion: a CUT voice reported the RELEASE
    /// time as the moment it could be dropped, so sixteenths stacked a dozen inaudible voices —
    /// filters and 2x-oversampled shapers, all still rendering — on top of the live one, and the
    /// audio thread paid for every one. Sparse notes hid it completely, which is exactly how it
    /// was described: distortion at 1/16, clean at one note a bar.
    #[test]
    fn a_cut_voice_is_reclaimed_in_milliseconds_not_in_a_release() {
        let ctx = OfflineAudioContext::new(1, 128, SR);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut params = HashMap::new();
        for (k, v) in [("harm", 0.5), ("rel", 0.5)] {
            params.insert(k.to_string(), v);
        }
        let job = Job {
            instr: "sub".into(), note: 36.0, vel: 1.0, params, dur: 0.5, sr: SR, t: 0.0,
            rev: false, strip: None, mix: None,
        };
        let mut cut = trig(&c, &job).expect("sub builds");
        let mut released = trig(&c, &job).expect("sub builds");
        cut.cut(1.0);
        released.release(1.0);
        // rel 0.5 s: a released voice is legitimately held 5.2 s. A cut one is silent in 25 ms
        // and every source is stopped at 60 ms, so holding it beyond ~0.12 s is pure waste.
        assert!(cut.quiet_by(1.0) <= 1.0 + 0.15,
                "a cut voice is held {:.2} s — dense patterns will drown in dead graph",
                cut.quiet_by(1.0) - 1.0);
        assert!(released.quiet_by(1.0) > 1.0 + 1.0,
                "a RELEASED voice must still be given its full tail, not the cut shortcut");
    }

    /// A cut SCHEDULED AHEAD must not touch the note that is still sounding.
    ///
    /// The 2026-08-15 distortion, reduced to one assertion. The engine schedules a lane's cut up
    /// to 160 ms in the future (the lookahead), and the vendored crate evaluated the cut's
    /// setTargetAtTime curve BEFORE its start time — exp of a large positive number — poisoning
    /// the gain to f32::MAX, so every retriggered note played at full-scale saturation until its
    /// cut arrived. Captured off the device as a ±0.92 square at note pitch; fixed in the fork
    /// (param.rs, the two VOLT FORK guards). The old cut test only measured AFTER the cut time,
    /// which is precisely the region the bug spared — this one asserts the WHOLE render.
    #[test]
    fn a_scheduled_cut_does_not_deafen_the_note_before_it_lands() {
        let mut ctx = OfflineAudioContext::new(1, (SR as usize) * 2, SR);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut params = HashMap::new();
        for (k, v) in [("harm", 0.15), ("rel", 0.06)] {
            params.insert(k.to_string(), v);
        }
        let job = Job {
            instr: "sub".into(), note: 36.0, vel: 0.8, params, dur: 0.5, sr: SR, t: 0.05,
            rev: false, strip: None, mix: None,
        };
        let mut v = trig(&c, &job).expect("sub builds");
        v.connect(&ctx.destination());
        v.cut(0.5); // scheduled 450 ms ahead of the audio, like every lookahead cut
        let buf = ctx.start_rendering_sync();
        let d = buf.get_channel_data(0);
        let peak = d.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            peak < 1.5,
            "a future cut blew the gain up mid-note: peak {peak:e} — the param.rs fork guard is gone"
        );
        assert!(peak > 0.3, "the note itself should still sound before the cut: peak {peak}");
    }

    #[test]
    fn release_fades_over_the_instruments_own_time() {
        // rel = 0.2 s, so the fade uses a 66 ms time constant and the sources stop at t + 0.9 s.
        // The point of the test is the shape: audible just after the release, gone well before the
        // sources are stopped — i.e. it is the envelope doing the work, not the hard stop.
        let b = render(Some(("release", 0.3)), 0.2);
        assert!(rms(&b, 0.31, 0.36) > 0.1, "release cut the note dead instead of fading it");
        assert!(rms(&b, 0.75, 1.0) < 1e-3, "release never got there: {}", rms(&b, 0.75, 1.0));
        // ...and a longer release must still be audible where a short one is already gone.
        let short = render(Some(("release", 0.02)), 0.02);
        assert!(
            rms(&b, 0.36, 0.42) > rms(&short, 0.36, 0.42) * 4.0,
            "release time is not being used: long {} vs short {}",
            rms(&b, 0.36, 0.42),
            rms(&short, 0.36, 0.42)
        );
    }

    /// A voice is one-shot. `stopAllVoices()` cuts everything on the way out, and a cut that
    /// followed a release would re-schedule a faster fade from wherever the first one had got to.
    #[test]
    fn ending_a_voice_twice_does_nothing_the_second_time() {
        let mut ctx = OfflineAudioContext::new(1, 1024, SR);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut params = HashMap::new();
        params.insert("harm".to_string(), 0.5f32);
        let job = Job { instr: "sub".into(), note: 36.0, vel: 1.0, params, dur: 0.5, sr: SR, t: 0.0, mix: None,
            rev: false, strip: None };
        let mut v = trig(&c, &job).unwrap();
        assert!(!v.dead());
        v.release(0.01);
        assert!(v.dead());
        v.cut(0.02); // must be a no-op
        assert!(v.dead());
    }
}

#[cfg(test)]
mod panic_tests {
    use super::*;
    use web_audio_api::context::OfflineAudioContext;

    /// `Job::p()` panics on a missing parameter, and that is deliberate — a silently-defaulted
    /// parameter is the one bug a null test can never catch, because both engines still make *a*
    /// sound. This test exists to pin that behaviour down, because the live engine now depends on
    /// knowing it: `native/desktop/src/engine.rs` catches the unwind rather than letting it kill
    /// the audio thread. If `p()` ever stops panicking, that catch becomes dead code and this
    /// fails, which is the reminder to go and look at it.
    #[test]
    fn a_missing_parameter_still_panics_loudly() {
        let ctx = OfflineAudioContext::new(1, 128, 44100.0);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        // `sub` needs `harm`. An instrument from a song the port does not fully know looks exactly
        // like this — and before the catch, one of these took the whole ASIO engine down for good.
        let job = Job {
            instr: "sub".into(),
            note: 36.0,
            vel: 1.0,
            params: HashMap::new(),
            dur: 0.5,
            sr: 44100.0,
            t: 0.0,
            rev: false,
            strip: None,
            mix: None,
        };
        // map to () so the assertion does not need Voice: Debug — the voice itself is not the point
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            trig(&c, &job).map(|_| ()).map_err(|e| e)
        }));
        assert!(caught.is_err(), "p() no longer panics — see engine.rs's catch_unwind");
        // ...and the message has to name the parameter and the instrument, because that string is
        // what now reaches Mickey's status line.
        let e = caught.unwrap_err();
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "(not a String)".into());
        assert!(msg.contains("harm") && msg.contains("sub"), "unhelpful panic message: {msg}");
    }

    /// An instrument nobody has ported returns an Err rather than panicking — the ordinary path,
    /// and the one the engine reports without any unwinding at all.
    #[test]
    fn an_unported_instrument_is_an_error_not_a_panic() {
        let ctx = OfflineAudioContext::new(1, 128, 44100.0);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        // Every real instrument is ported as of 2026-08-14, so the unported case needs an
        // instrument that does not exist — which is also what a future song format could send.
        let job = Job {
            instr: "theremin".into(),
            note: 48.0,
            vel: 1.0,
            params: HashMap::new(),
            dur: 0.5,
            sr: 44100.0,
            t: 0.0,
            rev: false,
            strip: None,
            mix: None,
        };
        match trig(&c, &job) {
            Err(e) => assert!(e.contains("theremin"), "error should name the instrument: {e}"),
            Ok(_) => panic!("an unknown instrument must not silently succeed"),
        };
    }
}
