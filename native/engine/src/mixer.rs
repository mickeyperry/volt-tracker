//! The channel strip — `ensureChans()` and `applyMix()` from beta2.html, in Rust.
//!
//! Same rule as the instruments: **this is a transliteration, not a rewrite.** Every node below has
//! a twin in `ensureChans()` (beta2.html:1097) in the same order with the same numbers, and every
//! line of [`Channel::apply`] has a twin in `applyMix()` (beta2.html:1117). `tests/parity.test.js`
//! renders a note through the browser's strip and through this one and subtracts them, exactly as
//! it does for a kick.
//!
//! Why this exists at all: today the native engine plays voices straight at the destination. There
//! is no fader, no EQ, and — the reason this is the biggest job on the roadmap — **nowhere for a
//! VST effect to live.** A plugin is a node in an insert chain; the insert chain is here.
//!
//! [`Master`] is the other half — `buildGraph()` (beta2.html:1049): the ping-pong delay bus, the
//! reverb bus, the master fader, its insert slot and the safety limiter. Between them a [`Mixer`]
//! is the whole thing the browser has, minus the metering taps.
//!
//! What is deliberately *not* here yet:
//!   * the per-channel analyser, which is a metering tap and changes no audio,
//!   * mute/solo and duck, which are applied by callers in the browser too, not by `applyMix`,
//!   * the 15 insert effects (stage D) — the slot they go in is `fx_in`/`fx_out`,
//!   * **plugin delay compensation.** Nothing in the strip reports latency yet, so there is nothing
//!     to compensate; the moment a VST sits in an insert there will be, and the engineering bible
//!     (`03_DSP_GRAPH`, "latency compensation") is unambiguous that fan-in is where it must happen —
//!     which in this graph means the master and the two send buses. [`Master::sum`] is deliberately
//!     the one place every channel converges, so that when PDC arrives it has exactly one home.

use serde::Deserialize;
use web_audio_api::context::BaseAudioContext;
use web_audio_api::node::{
    AnalyserNode, AudioNode, BiquadFilterNode, BiquadFilterType, ChannelCountMode, DelayNode,
    DynamicsCompressorNode, GainNode, OverSampleType, StereoPannerNode, WaveShaperNode,
};
use web_audio_api::AudioBuffer;

use crate::fx::{FxChain, FxSet};
use crate::{drive_curve, seed_rnd};

/// The shortest the ping-pong delay may ever be set to, in seconds.
///
/// This is a SAFETY floor, not a taste decision: the delay pair sits inside a 0.42 feedback loop,
/// and at a delay time of zero that loop is an amplifier with no time constant. 20 ms is shorter
/// than any tempo-derived value the app produces (a 1/8 dotted at 300 bpm is 150 ms) so it never
/// changes how anything sounds — it only refuses the value that detonates.
const DELAY_MIN: f32 = 0.02;

/// `fltHz` — beta2.html:945. The filter knob is 0..1 and the sweep is exponential, 60 Hz to 18 kHz,
/// because a linear frequency control spends nine tenths of its travel in the top octave.
pub fn flt_hz(x: f32) -> f32 {
    60.0 * 300f32.powf(x.clamp(0.0, 1.0))
}

/// The three EQ bands, as they sit in the song file.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Eq {
    #[serde(default)]
    pub l: f32,
    #[serde(default)]
    pub m: f32,
    #[serde(default)]
    pub h: f32,
}

/// One track's mixer settings — `mkTs()` in beta2.html:996.
///
/// Every field carries the same default the browser reaches for when the key is missing, because
/// `applyMix` is written defensively (`ts.pan||0`, `ts.fltF==null?1:ts.fltF`) and songs saved by
/// older builds genuinely do lack fields. A default that disagrees with the browser's would move a
/// fader on a song Mickey saved last year, which is rule 3.
#[derive(Debug, Clone, Deserialize)]
pub struct TrackSet {
    #[serde(default = "vol_default")]
    pub vol: f32,
    #[serde(default)]
    pub pan: f32,
    /// Send levels. Named `dly`/`rvb` in the song file.
    #[serde(default)]
    pub dly: f32,
    #[serde(default)]
    pub rvb: f32,
    #[serde(default)]
    pub eq: Eq,
    /// Compressor amount, 0..1 — one knob standing in for threshold and ratio together.
    #[serde(default)]
    pub cmp: f32,
    /// Drive amount, 0..1.
    #[serde(default)]
    pub drv: f32,
    #[serde(rename = "fltF", default = "one")]
    pub flt_f: f32,
    #[serde(rename = "fltQ", default)]
    pub flt_q: f32,
    /// The insert chain, exactly as it sits in the song file. Rode along in `eng_mix`'s payload
    /// from the start (serde ignored it); stage D reads it.
    #[serde(default)]
    pub fx: Vec<FxSet>,
    /// Sidechain duck: how far this channel dips when its trigger fires (0 = no duck).
    #[serde(default)]
    pub duck: f32,
    /// Which track triggers the dip. Negative is the legacy behaviour: any kick, wherever it is.
    #[serde(rename = "duckSrc", default = "neg_one")]
    pub duck_src: f32,
    /// Release knob, 0..1 — the browser turns it into seconds as `.03 + duckRel * .5`.
    #[serde(rename = "duckRel", default = "duck_rel_default")]
    pub duck_rel: f32,
    /// Mute and solo. The browser mutes by not *scheduling* notes, so these never needed to exist
    /// here — until a channel's sound lived in THIS mixer (a plugin, or a ported instrument under
    /// a plugin insert), at which point muting it did nothing at all. They ride in the same
    /// trackset `eng_mix` always sent; [`Mixer::apply`] turns them into a fader gate, which is
    /// better than not scheduling would be anyway: it works mid-note, in both directions.
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
}

fn neg_one() -> f32 {
    -1.0
}
fn duck_rel_default() -> f32 {
    0.16
}

fn vol_default() -> f32 {
    0.85
}
fn one() -> f32 {
    1.0
}

impl Default for TrackSet {
    fn default() -> Self {
        Self {
            vol: vol_default(),
            pan: 0.0,
            dly: 0.0,
            rvb: 0.0,
            eq: Eq::default(),
            cmp: 0.0,
            drv: 0.0,
            flt_f: 1.0,
            flt_q: 0.0,
            fx: Vec::new(),
            duck: 0.0,
            duck_src: -1.0,
            duck_rel: 0.16,
            mute: false,
            solo: false,
        }
    }
}

/// One channel of the mixer. `chans[t]` in beta2.html.
///
/// The signal path, and it is the browser's path node for node:
///
/// ```text
///   in ─ eqL ─ eqM ─ eqH ─ flt ─ fxIn ─ fxOut ─ shp ─ cmp ─ pan ─ vg ─ duck ─┬─ (master)
///                                 └── insert FX, and where a VST goes ──┘     ├─ ds → delay bus
///                                                                             └─ rs → reverb bus
/// ```
///
/// The fader (`vg`) sits *after* everything, so it only ever changes loudness — moving it must not
/// change how hard the compressor works or how much the drive bites. That ordering is a mixing
/// decision, not an accident, and it is why the sends are taken after it.
pub struct Channel {
    /// Where voices connect. `ch.in` in the browser (`g` in `ensureChans`).
    pub input: GainNode,
    eq_l: BiquadFilterNode,
    eq_m: BiquadFilterNode,
    eq_h: BiquadFilterNode,
    flt: BiquadFilterNode,
    /// The insert chain's two ends, wired straight through until something is put between them.
    /// A ported effect — or a VST3 plugin — is spliced in here and nowhere else.
    pub fx_in: GainNode,
    pub fx_out: GainNode,
    shp: WaveShaperNode,
    cmp: DynamicsCompressorNode,
    pan: StereoPannerNode,
    /// The fader.
    vg: GainNode,
    /// Post-fader, pre-send: what sidechain ducking pulls down. The output of the strip.
    pub duck: GainNode,
    /// Delay send.
    pub ds: GainNode,
    /// Reverb send.
    pub rs: GainNode,
    /// `ch._drv` — the last drive amount applied. The shaper's curve is 512 floats and rebuilding
    /// it on every mix update, sixty times a second under an automation lane, is real work for no
    /// change in sound, so the browser skips it when the value has not moved. So does this.
    drv: f32,
    /// `ch.an` — the metering tap, off `duck` exactly where the browser hangs its analyser.
    /// This is what the meters and the spectrum panel read when the sound they show lives here
    /// rather than in the webview (a plugin channel, or "ASIO only").
    an: AnalyserNode,
    /// The insert chain between `fx_in` and `fx_out` — stage D. See `fx.rs`.
    fx: FxChain,
    /// The VST-effect insert: a second straight-through pair AFTER the ported chain
    /// (`fx_out → pfx_in → pfx_out → shp`), so a plugin effect and the ported effects coexist
    /// without either rebuild touching the other. The engine owns the plugin node and splices it
    /// between these via [`Mixer::set_plugin_insert`].
    pfx_in: GainNode,
    pfx_out: GainNode,
    /// This channel's sidechain settings, held from the last [`Channel::apply`] so a duck trigger
    /// can be answered at note-schedule time without a snapshot lookup.
    duck_amt: f32,
    duck_src: f32,
    duck_rel: f32,
}

impl Channel {
    /// `ensureChans()` — build one strip. Fully wired, but with nothing applied: call
    /// [`Channel::apply`] before playing anything through it, exactly as the browser does.
    pub fn new<C: BaseAudioContext>(ctx: &C) -> Self {
        let input = ctx.create_gain();
        let duck = ctx.create_gain();
        let ds = ctx.create_gain();
        let rs = ctx.create_gain();

        let mut eq_l = ctx.create_biquad_filter();
        eq_l.set_type(BiquadFilterType::Lowshelf);
        eq_l.frequency().set_value(110.0);
        let mut eq_m = ctx.create_biquad_filter();
        eq_m.set_type(BiquadFilterType::Peaking);
        eq_m.frequency().set_value(950.0);
        eq_m.q().set_value(0.9);
        let mut eq_h = ctx.create_biquad_filter();
        eq_h.set_type(BiquadFilterType::Highshelf);
        eq_h.frequency().set_value(6500.0);

        let mut flt = ctx.create_biquad_filter();
        flt.set_type(BiquadFilterType::Lowpass);
        flt.frequency().set_value(18000.0);
        flt.q().set_value(0.0);

        let fx_in = ctx.create_gain();
        let fx_out = ctx.create_gain();

        let mut shp = ctx.create_wave_shaper();
        shp.set_oversample(OverSampleType::X2);

        let cmp = ctx.create_dynamics_compressor();
        // **Chrome's compressor makes a mono signal stereo, and the spec says it should not.**
        //
        // The spec gives DynamicsCompressorNode channelCountMode "clamped-max", which for a mono
        // input computes min(2, 1) = 1 channel — mono in, mono out. Chrome outputs two channels
        // regardless, and this crate follows the spec, and that one-word difference is worth 3 dB
        // of level: the strip's panner is next, and a StereoPanner applies the equal-power MONO law
        // (x0.7071 to each side) to a mono input and the STEREO law (unity at centre) to a stereo
        // one. Measured against Chrome: mono → panner is -3 dB, mono → compressor → panner is 0 dB.
        //
        // Every VOLT instrument is mono, so this is not an edge case — it is every note. Forcing
        // the count to 2 explicitly reproduces what Mickey's songs already sound like, which is the
        // only bar that matters here. (Up-mixing 1 → 2 copies the channel at unity, per spec, so
        // nothing else about it is a guess.)
        cmp.set_channel_count_mode(ChannelCountMode::Explicit);
        cmp.set_channel_count(2);
        let pan = ctx.create_stereo_panner();
        let vg = ctx.create_gain();

        // The meter/spectrum tap. 2048-point FFT because that is what the browser's spectrum panel
        // sets on the channel it watches (`SPEC.fft`), and the byte scaling defaults (-100..-30 dB)
        // are the spec's, which are also the browser's.
        let an = ctx.create_analyser();

        let pfx_in = ctx.create_gain();
        let pfx_out = ctx.create_gain();

        input.connect(&eq_l);
        eq_l.connect(&eq_m);
        eq_m.connect(&eq_h);
        eq_h.connect(&flt);
        flt.connect(&fx_in);
        fx_in.connect(&fx_out);
        fx_out.connect(&pfx_in);
        pfx_in.connect(&pfx_out);
        pfx_out.connect(&shp);
        shp.connect(&cmp);
        cmp.connect(&pan);
        pan.connect(&vg);
        vg.connect(&duck);
        duck.connect(&ds);
        duck.connect(&rs);
        duck.connect(&an);

        Self {
            input, eq_l, eq_m, eq_h, flt, fx_in, fx_out, shp, cmp, pan, vg, duck, ds, rs,
            drv: 0.0, an, fx: FxChain::default(), pfx_in, pfx_out,
            duck_amt: 0.0, duck_src: -1.0, duck_rel: 0.16,
        }
    }

    /// `applyMix()`'s per-track body — beta2.html:1117.
    ///
    /// Deliberately a wholesale re-apply rather than a diff: the browser re-applies every parameter
    /// of every channel on any mixer edit, it costs microseconds, and it means the graph can never
    /// drift out of step with the song. The one exception is the drive curve, which is expensive
    /// enough to be worth the guard and cheap enough to guard correctly.
    pub fn apply(&mut self, ts: &TrackSet) {
        self.vg.gain().set_value(ts.vol);
        self.ds.gain().set_value(ts.dly);
        self.rs.gain().set_value(ts.rvb);

        self.eq_l.gain().set_value(ts.eq.l);
        self.eq_m.gain().set_value(ts.eq.m);
        self.eq_h.gain().set_value(ts.eq.h);

        self.flt.frequency().set_value(flt_hz(ts.flt_f));
        self.flt.q().set_value(ts.flt_q * 20.0);

        self.pan.pan().set_value(ts.pan);

        // One knob, two parameters: at 0 the ratio is 1:1 and the threshold is 0 dB, which is a
        // compressor that cannot compress. It is still in circuit, and still costs its 6 ms of
        // spec-fixed lookahead — that latency is part of the sound of the mix and is why the
        // metronome has to be delayed to match (MET.lat in the browser).
        let c = ts.cmp;
        self.cmp.threshold().set_value(-30.0 * c);
        self.cmp.ratio().set_value(1.0 + 7.0 * c);
        self.cmp.knee().set_value(6.0);
        self.cmp.attack().set_value(0.003);
        self.cmp.release().set_value(0.12);

        let d = ts.drv;
        if d != self.drv {
            self.drv = d;
            if d > 0.0 {
                self.shp.set_curve(drive_curve(d));
            } else {
                // `ch.shp.curve = null` — a straight wire, not an identity curve. See clear_curve
                // in the vendored crate for why the distinction matters.
                self.shp.clear_curve();
            }
        }

        // Held for the duck triggers — see [`Mixer::duck`]. Config only; the envelope itself is
        // scheduled per note, never from apply.
        self.duck_amt = ts.duck;
        self.duck_src = ts.duck_src;
        self.duck_rel = ts.duck_rel;
    }

    /// The insert chain — `buildFx(ch, ts.fx)` plus tuning. Separate from [`Channel::apply`]
    /// because building an effect needs the context and `apply` never did. Returns the effect
    /// types it had to skip (unported), for the caller to report.
    pub fn apply_fx<C: BaseAudioContext>(&mut self, ctx: &C, list: &[FxSet]) -> Vec<String> {
        let Channel { fx, fx_in, fx_out, .. } = self;
        fx.apply(ctx, fx_in, fx_out, list)
    }

    /// The loudest sample in the analyser's current window — the meter's number.
    pub fn peak(&mut self, scratch: &mut Vec<f32>) -> f32 {
        scratch.resize(self.an.fft_size(), 0.0);
        self.an.get_float_time_domain_data(scratch);
        scratch.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// The strip's output. Connect it where the browser connects `duckG` — the master bus.
    pub fn connect<N: AudioNode>(&self, dest: &N) {
        self.duck.connect(dest);
    }
}

/// The master bus — `buildGraph()` in beta2.html:1049, everything after the channel strips.
///
/// ```text
///   channels ─┬──────────────────────────────────────────► master ─ mFx ─ limiter ─► out
///             ├─ ds ─► delay bus (ping-pong) ─────────────►   ▲
///             └─ rs ─► reverb bus (convolver) ────────────────┘
/// ```
///
/// **The two send buses return to the master fader, not past it**, exactly as the browser has it
/// (`dout.connect(master)`, `rout.connect(master)`). That means the master fader rides the wet
/// signal too — pull the master down and the reverb comes with it, which is what anybody expects
/// and is not true of a design that returns after the fader.
///
/// The limiter is a **safety** limiter and not a mix compressor: -3 dB threshold, ratio 12, a
/// 1.5 ms attack. It is there so a kick and a snare landing together cannot clip the output, not to
/// glue anything, and its settings are the browser's to the digit.
pub struct Master {
    /// Where every channel's `duck` arrives, and the one place the whole mix converges.
    ///
    /// **This is where plugin delay compensation will go**, and it is a named node rather than
    /// `master` itself so that there is somewhere to put it. The engineering bible is explicit
    /// (`03_DSP_GRAPH`): compensation belongs at fan-in, delaying the early arrivals to match the
    /// latest. Nothing reports latency yet, so today this is a plain unity gain.
    sum: GainNode,
    master: GainNode,
    /// The master insert slot — `MCH.fxIn`/`MCH.fxOut`. Wired straight through until stage D puts
    /// something between them.
    pub m_fx_in: GainNode,
    pub m_fx_out: GainNode,
    /// Never read after construction — but dropping it would take the limiter out of the graph,
    /// so holding it IS the job. It will be read the day the master GR readout is published back
    /// to the UI, which is what `masterComp` is exposed for in the browser.
    #[allow(dead_code)]
    limiter: DynamicsCompressorNode,
    /// The last thing before the speakers — `spkOut`. Kept separate for the same reason the browser
    /// keeps it: it is the only node "ASIO only" may touch, and muting it must not silence metering.
    pub spk_out: GainNode,
    /// Send bus inputs. Channels connect their `ds`/`rs` here.
    pub dly_in: GainNode,
    pub rvb_in: GainNode,
    /// The two ping-pong taps, whose time follows the tempo.
    dly_l: DelayNode,
    dly_r: DelayNode,
    /// `masterAn` — the mix's metering tap, off the limiter exactly as the browser taps `comp`.
    an: AnalyserNode,
    /// The master insert chain — `song.masterFx`, between `m_fx_in` and `m_fx_out`.
    fx: FxChain,
    /// The master's VST-effect insert, after the ported chain and BEFORE the safety limiter —
    /// a mastering plugin gets the whole mix and the limiter still catches whatever it does.
    pfx_in: GainNode,
    pfx_out: GainNode,
    /// Held so the graph keeps them alive; the crate drops a node's audio when its handle goes.
    _bus: Vec<Box<dyn AudioNode>>,
}

impl Master {
    /// Build the master and both send buses.
    pub fn new<C: BaseAudioContext>(ctx: &C) -> Self {
        Self::build(ctx, true, true)
    }

    /// As [`new`](Master::new), with either send bus optionally left out. See [`Mixer::new_with`].
    pub fn build<C: BaseAudioContext>(ctx: &C, want_delay: bool, want_reverb: bool) -> Self {
        let sample_rate = ctx.sample_rate();
        let mut keep: Vec<Box<dyn AudioNode>> = Vec::new();

        let sum = ctx.create_gain();
        let master = ctx.create_gain();
        master.gain().set_value(0.75);

        let m_fx_in = ctx.create_gain();
        let m_fx_out = ctx.create_gain();

        let limiter = ctx.create_dynamics_compressor();
        limiter.threshold().set_value(-3.0);
        limiter.knee().set_value(2.0);
        limiter.ratio().set_value(12.0);
        limiter.attack().set_value(0.0015);
        limiter.release().set_value(0.08);

        let spk_out = ctx.create_gain();
        let an = ctx.create_analyser();
        let pfx_in = ctx.create_gain();
        let pfx_out = ctx.create_gain();

        sum.connect(&master);
        master.connect(&m_fx_in);
        m_fx_in.connect(&m_fx_out);
        m_fx_out.connect(&pfx_in);
        pfx_in.connect(&pfx_out);
        pfx_out.connect(&limiter);
        limiter.connect(&spk_out);
        limiter.connect(&an);

        // ---- the ping-pong delay bus -------------------------------------------------------
        // Left feeds right feeds left, each through its own feedback gain, with the two taps panned
        // hard-ish apart. The high-pass in front is what stops a delay turning a mix to mud: only
        // the part of the signal above 280 Hz is repeated.
        let dly_in = ctx.create_gain();
        let dly_l = ctx.create_delay(2.0);
        let dly_r = ctx.create_delay(2.0);
        // The times themselves are set from the song's tempo by `apply_delay`; the browser does
        // the same thing in `applyMix`.
        //
        // **"A delay bus with no time set is silent, not broken" was WRONG, and it was the worst
        // kind of wrong.** These two delays sit in a feedback loop with a gain of 0.42. At a delay
        // time of zero that loop has no delay to spread the feedback over, so every render quantum
        // multiplies what came before: measured with `examples/denseprobe` on 2026-08-14 — peak
        // **2,272,955**, a DC offset of 415, and RMS gone to NaN inside four seconds, from a single
        // reese note. Anything with a delay send routes into this. Start them at a musical value
        // instead of at the detonator, and `set_delay_time` refuses zero from now on.
        dly_l.delay_time().set_value(DELAY_MIN);
        dly_r.delay_time().set_value(DELAY_MIN);
        let fb1 = ctx.create_gain();
        let fb2 = ctx.create_gain();
        fb1.gain().set_value(0.42);
        fb2.gain().set_value(0.42);
        let mut lhp = ctx.create_biquad_filter();
        lhp.set_type(BiquadFilterType::Highpass);
        lhp.frequency().set_value(280.0);
        let pl = ctx.create_stereo_panner();
        let pr = ctx.create_stereo_panner();
        pl.pan().set_value(-0.65);
        pr.pan().set_value(0.65);
        let dout = ctx.create_gain();
        dout.gain().set_value(0.9);

        if want_delay {
            dly_in.connect(&lhp);
            lhp.connect(&dly_l);
            dly_l.connect(&pl);
            pl.connect(&dout);
            dly_l.connect(&fb1);
            fb1.connect(&dly_r);
            dly_r.connect(&pr);
            pr.connect(&dout);
            dly_r.connect(&fb2);
            fb2.connect(&dly_l);
            dout.connect(&master);
        }

        // ---- the reverb bus ----------------------------------------------------------------
        let rvb_in = ctx.create_gain();
        let mut cv = ctx.create_convolver();
        cv.set_buffer(reverb_ir(ctx, sample_rate));
        let mut rhp = ctx.create_biquad_filter();
        rhp.set_type(BiquadFilterType::Highpass);
        rhp.frequency().set_value(300.0);
        let rout = ctx.create_gain();
        rout.gain().set_value(0.7);

        if want_reverb {
            rvb_in.connect(&rhp);
            rhp.connect(&cv);
            cv.connect(&rout);
            rout.connect(&master);
        }

        // Everything with no field of its own still has to outlive this function.
        keep.push(Box::new(lhp));
        keep.push(Box::new(fb1));
        keep.push(Box::new(fb2));
        keep.push(Box::new(pl));
        keep.push(Box::new(pr));
        keep.push(Box::new(dout));
        keep.push(Box::new(rhp));
        keep.push(Box::new(cv));
        keep.push(Box::new(rout));

        Master {
            sum,
            master,
            m_fx_in,
            m_fx_out,
            limiter,
            spk_out,
            dly_in,
            rvb_in,
            dly_l,
            dly_r,
            an,
            fx: FxChain::default(),
            pfx_in,
            pfx_out,
            _bus: keep,
        }
    }

    /// The master insert chain — `buildFx(MCH, song.masterFx)`. Returns unported types.
    pub fn apply_fx<C: BaseAudioContext>(&mut self, ctx: &C, list: &[FxSet]) -> Vec<String> {
        let Master { fx, m_fx_in, m_fx_out, .. } = self;
        fx.apply(ctx, m_fx_in, m_fx_out, list)
    }

    /// Where a channel's output goes.
    pub fn channel_input(&self) -> &GainNode {
        &self.sum
    }

    /// The master fader, 0..1. `master.gain.value` in the browser, default 0.75.
    pub fn set_volume(&mut self, v: f32) {
        self.master.gain().set_value(v);
    }

    /// The ping-pong delay's time, in seconds per tap — `applyMix()` derives it from the tempo.
    pub fn set_delay_time(&mut self, secs: f32) {
        // Never zero, never negative, never NaN — see the note where these nodes are built. A
        // snapshot from an older UI (or any future caller that forgets the field) deserializes
        // `delayTime` to 0.0, and 0.0 through a 0.42 feedback path is not a short echo, it is a
        // runaway that takes the whole mix with it.
        let secs = if secs.is_finite() { secs.max(DELAY_MIN) } else { DELAY_MIN };
        self.dly_l.delay_time().set_value(secs);
        self.dly_r.delay_time().set_value(secs);
    }

    /// Connect the finished mix to the hardware.
    pub fn connect<N: AudioNode>(&self, dest: &N) {
        self.spk_out.connect(dest);
    }

    /// Silence the speakers without touching the mix — "ASIO only" in reverse, and the one node
    /// that may be muted. See `spkOut` in beta2.html.
    pub fn set_speakers(&mut self, on: bool) {
        self.spk_out.gain().set_value(if on { 1.0 } else { 0.0 });
    }
}

/// The whole mixer: channel strips plus the master they feed.
///
/// Channels are made **on demand and never destroyed**, exactly as `ensureChans(n)` does in the
/// browser: a song that grows a track gets a strip, a song that loses one leaves a silent strip
/// behind. That is deliberate — tearing a strip out from under a voice that is still sounding is a
/// click, and an idle strip costs nothing but a few unused nodes.
pub struct Mixer {
    chans: Vec<Channel>,
    pub master: Master,
}

impl Mixer {
    pub fn new<C: BaseAudioContext>(ctx: &C) -> Self {
        Mixer { chans: Vec::new(), master: Master::new(ctx) }
    }

    /// The same, with either send bus left unbuilt. **A bisection handle, not a feature** — the
    /// delay bus contains a feedback cycle and the reverb bus a 1.9-second convolution, and when
    /// the app hung on Play those were the two things in the graph that a live callback treats
    /// differently from an offline render. `examples/livemix.rs` uses it to say which.
    pub fn new_with<C: BaseAudioContext>(ctx: &C, delay: bool, reverb: bool) -> Self {
        Mixer { chans: Vec::new(), master: Master::build(ctx, delay, reverb) }
    }

    /// `ensureChans(n)` — grow to `n` strips, wiring each into the master and both send buses.
    pub fn ensure<C: BaseAudioContext>(&mut self, ctx: &C, n: usize) {
        while self.chans.len() < n {
            let ch = Channel::new(ctx);
            ch.connect(self.master.channel_input());
            ch.ds.connect(&self.master.dly_in);
            ch.rs.connect(&self.master.rvb_in);
            self.chans.push(ch);
        }
    }

    /// Where a voice on `track` should play. `None` if that strip has not been made yet — callers
    /// should `ensure` first; this never creates one, because it has no context to create it with.
    pub fn input(&self, track: usize) -> Option<&GainNode> {
        self.chans.get(track).map(|c| &c.input)
    }

    /// The insert slot for `track`, where a ported effect or a VST goes.
    pub fn insert(&self, track: usize) -> Option<(&GainNode, &GainNode)> {
        self.chans.get(track).map(|c| (&c.fx_in, &c.fx_out))
    }

    /// `applyMix()` — push a whole mixer snapshot in. Wholesale rather than a diff, for the reason
    /// [`Channel::apply`] gives: the browser does the same, it costs microseconds, and it means the
    /// graph cannot drift out of step with the song.
    ///
    /// Returns the effect types it could not build (the unported ones), deduplicated, so the
    /// caller can say so once instead of the difference being silent.
    pub fn apply<C: BaseAudioContext>(&mut self, ctx: &C, state: &MixState) -> Vec<String> {
        let mut skipped: Vec<String> = Vec::new();
        self.ensure(ctx, state.tracks.len());
        // `trackAudible()` in beta2.html:3185 — mute wins, and one solo anywhere silences every
        // track that is not soloed. The browser expresses this by not scheduling notes; a channel
        // whose sound lives HERE (a plugin, or ported instruments under a plugin insert) has no
        // "don't schedule" to lean on, so it is a gate on the fader instead. The fader and not the
        // input, so a mute is instant both ways — a note already ringing dies NOW, and un-muting
        // mid-pattern brings the channel back mid-note instead of at its next note-on.
        // (The webview pre-folds poly-lane leadership into `mute` and sends `solo:false` — a lane
        // follows its leader, and this mixer cannot know who leads whom. The formula still stands
        // on its own for any caller that sends the raw flags.)
        let soloed = state.tracks.iter().any(|t| t.solo);
        for (i, ts) in state.tracks.iter().enumerate() {
            if let Some(ch) = self.chans.get_mut(i) {
                ch.apply(ts);
                if ts.mute || (soloed && !ts.solo) {
                    ch.vg.gain().set_value(0.0);
                }
                for k in ch.apply_fx(ctx, &ts.fx) {
                    if !skipped.contains(&k) {
                        skipped.push(k);
                    }
                }
            }
        }
        self.master.set_volume(state.master);
        self.master.set_delay_time(state.delay_time);
        for k in self.master.apply_fx(ctx, &state.master_fx) {
            if !skipped.contains(&k) {
                skipped.push(k);
            }
        }
        skipped
    }

    /// One peak per strip — what the channel meters read when the channel's sound lives here.
    /// The loudest sample the MASTER is currently putting out, tapped after the safety limiter
    /// exactly where the browser's meter taps `comp`.
    ///
    /// Its whole job is to answer one question the render-load meter cannot: is the engine
    /// sending the device something above full scale? A converter has nowhere to put 1.2, so it
    /// squares the top off — harsh distortion that arrives with the notes, sounds nothing like a
    /// dropout, and leaves no trace in any CPU number.
    ///
    /// It is also the mixer's MASTER METER while the engine owns the sound: `peaks` is indexed by
    /// track and the master has no track number to sit at, so without reporting this the master
    /// strip had no source at all and read a flat nothing while every channel moved — which looks
    /// exactly like "the channels are not reaching the master" (2026-08-15). They were.
    pub fn master_peak(&mut self, scratch: &mut Vec<f32>) -> f32 {
        scratch.resize(self.master.an.fft_size(), 0.0);
        self.master.an.get_float_time_domain_data(scratch);
        scratch.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    pub fn peaks(&mut self, scratch: &mut Vec<f32>) -> Vec<f32> {
        self.chans.iter_mut().map(|c| c.peak(scratch)).collect()
    }

    /// Sidechain duck — `duck(t, src, isKick)` from beta2.html:2105, the same numbers.
    ///
    /// Driven by NOTES, not by listening to audio, exactly as the browser does it: the scheduler
    /// knows every hit before it sounds, so the gain curve lands sample-accurately on the note.
    /// Called by the engine as it schedules each note; without this the native strips — which is
    /// to say every VST channel — never pumped at all, while the webview's copy of the mix did.
    pub fn duck(&mut self, t: f64, src: usize, is_kick: bool) {
        for (tr, ch) in self.chans.iter_mut().enumerate() {
            if !(ch.duck_amt > 0.0) || tr == src {
                continue; // a track never ducks itself
            }
            let wanted = if ch.duck_src < 0.0 { is_kick } else { ch.duck_src as usize == src };
            if !wanted {
                continue;
            }
            let rel = (0.03 + ch.duck_rel * 0.5) as f64;
            let hold = rel * 0.7;
            let gp = ch.duck.gain();
            gp.cancel_scheduled_values(t);
            gp.set_target_at_time(1.0 - ch.duck_amt, t, 0.006); // down fast — the "thump"
            gp.set_target_at_time(1.0, t + hold, rel); // back up slowly — the "pump"
        }
    }

    /// Every duck gain back to unity, now — the panic's half of the sidechain, mirroring the
    /// browser's `panic()` resetting each `duckG`.
    pub fn duck_reset(&mut self, t: f64) {
        for ch in &mut self.chans {
            let gp = ch.duck.gain();
            gp.cancel_scheduled_values(t);
            gp.set_value_at_time(1.0, t);
        }
    }

    /// Splice a plugin-effect node into `track`'s insert, or restore the straight wire.
    ///
    /// The caller (the engine) owns the node and its plugin; this owns only the wiring. The node
    /// sits between `pfx_in` and `pfx_out` — after the ported chain, before the drive — so a
    /// Pro-Q hears the channel EQ'd and filtered, and the strip's own compressor still protects
    /// whatever the plugin puts out.
    pub fn set_plugin_insert<N: AudioNode>(&mut self, track: usize, node: Option<&N>) {
        match node {
            Some(n) => self.set_plugin_chain(track, &[n]),
            None => self.set_plugin_chain::<N>(track, &[]),
        }
    }

    /// The master's plugin insert — same contract, between the ported master chain and the
    /// safety limiter, so a mastering plugin hears everything and the limiter still has the
    /// last word.
    pub fn set_master_plugin_insert<N: AudioNode>(&mut self, node: Option<&N>) {
        match node {
            Some(n) => self.set_master_plugin_chain(&[n]),
            None => self.set_master_plugin_chain::<N>(&[]),
        }
    }

    /// Splice a whole CHAIN of plugin nodes into `track`'s insert, in order — `pfx_in → n0 → n1 →
    /// … → pfx_out` — or restore the straight wire with an empty slice. This is what "multiple
    /// VSTs on one channel" is at the graph level: nothing but wiring.
    pub fn set_plugin_chain<N: AudioNode>(&mut self, track: usize, nodes: &[&N]) {
        let Some(ch) = self.chans.get(track) else { return };
        Self::splice_chain(&ch.pfx_in, &ch.pfx_out, nodes);
    }

    /// The master's version of [`Mixer::set_plugin_chain`].
    pub fn set_master_plugin_chain<N: AudioNode>(&mut self, nodes: &[&N]) {
        Self::splice_chain(&self.master.pfx_in, &self.master.pfx_out, nodes);
    }

    fn splice_chain<N: AudioNode>(pfx_in: &GainNode, pfx_out: &GainNode, nodes: &[&N]) {
        // Outgoing edges only, on every party involved: `disconnect()` clears what a node feeds,
        // so clearing pfx_in and each plugin drops the whole old wiring whatever shape it had,
        // without touching the edges INTO pfx_in from the strip.
        pfx_in.disconnect();
        for n in nodes {
            n.disconnect();
        }
        match nodes.split_first() {
            None => {
                pfx_in.connect(pfx_out);
            }
            Some((first, rest)) => {
                pfx_in.connect(*first);
                let mut prev: &N = first;
                for n in rest {
                    prev.connect(*n);
                    prev = n;
                }
                prev.connect(pfx_out);
            }
        }
    }

    /// The byte spectrum of one strip (`Some(track)`) or of the master (`None`), sized to the
    /// analyser's bin count — the browser's `getByteFrequencyData`, across the wire.
    pub fn spectrum(&mut self, track: Option<usize>, out: &mut Vec<u8>) {
        let an = match track {
            Some(t) => match self.chans.get_mut(t) {
                Some(c) => &mut c.an,
                None => {
                    out.clear();
                    return;
                }
            },
            None => &mut self.master.an,
        };
        out.resize(an.frequency_bin_count(), 0);
        an.get_byte_frequency_data(out);
    }

    pub fn len(&self) -> usize {
        self.chans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chans.is_empty()
    }
}

/// One snapshot of everything the mixer can be told, as the webview sends it.
///
/// Deliberately the whole state rather than a delta — see [`Mixer::apply`].
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct MixState {
    /// One entry per track, in track order. `song.trackset` in the browser.
    pub tracks: Vec<TrackSet>,
    /// The master fader. 0.75 is the browser's default and what `buildGraph` sets.
    #[serde(rename = "master")]
    pub master: f32,
    /// Ping-pong delay time in seconds, already derived from the tempo by the caller — the browser
    /// owns the tempo, so it owns this arithmetic too.
    #[serde(rename = "delayTime")]
    pub delay_time: f32,
    /// `song.masterFx` — the master insert chain. Absent from an older UI means an empty chain.
    #[serde(rename = "masterFx", default)]
    pub master_fx: Vec<FxSet>,
    /// Monotonic counter from the UI, so a snapshot that overtakes a newer one can be discarded.
    ///
    /// Needed because `eng_mix` runs on a thread pool and the UI does not wait for it: two updates
    /// 40 ms apart can arrive in either order, and applying the older one last would leave a fader
    /// somewhere the user did not put it — and leave it there until the next edit. Absent (0) from
    /// an older UI means "no opinion", and is always accepted.
    #[serde(default)]
    pub seq: u64,
}

/// The reverb impulse — beta2.html:1114, seeded.
///
/// White noise with a `(1 - i/len)^2.7` decay over 1.9 seconds: not a real room, a cheap and
/// perfectly serviceable synthetic tail, and the same one the browser makes. **One generator across
/// both channels**, matching the browser exactly: two channels drawing from one stream get
/// different numbers, which is what keeps the reverb stereo. Seeding it is what lets this bus be
/// compared with the browser's at all.
fn reverb_ir<C: BaseAudioContext>(ctx: &C, sample_rate: f32) -> AudioBuffer {
    let len = (sample_rate as f64 * 1.9) as usize;
    let mut ir = ctx.create_buffer(2, len, sample_rate);
    let mut rnd = seed_rnd(0x2EFBAD);
    for ch in 0..2 {
        let data = ir.get_channel_data_mut(ch);
        for (i, s) in data.iter_mut().enumerate() {
            let decay = (1.0 - i as f64 / len as f64).powf(2.7);
            *s = ((rnd() * 2.0 - 1.0) * decay) as f32;
        }
    }
    ir
}

#[cfg(test)]
mod tests {
    use super::*;
    use web_audio_api::context::OfflineAudioContext;
    use web_audio_api::node::AudioScheduledSourceNode;

    /// The `eng_mix` payload, with the fields stage D added: per-track `fx` riding inside the
    /// trackset (it always did; serde used to ignore it) and `masterFx` alongside. Shape copied
    /// from what `nengMix()` sends, per the house rule: paste, don't edit to taste.
    #[test]
    fn a_mix_snapshot_with_fx_chains_deserialises() {
        let state: MixState = serde_json::from_str(
            r#"{
              "tracks":[
                {"vol":0.85,"pan":0,"dly":0,"rvb":0,"eq":{"l":0,"m":0,"h":0},"cmp":0,"drv":0,
                 "fltF":1,"fltQ":0,
                 "fx":[{"type":"echo","p":{"time":0.28,"fb":0.42},"on":true},
                       {"type":"crush","p":{"bits":6}}]},
                {"vol":0.85}
              ],
              "masterFx":[{"type":"limit","p":{"loud":35,"ceil":-0.3}}],
              "master":0.75,"delayTime":0.345,"seq":7
            }"#,
        )
        .expect("the eng_mix payload must deserialise");
        assert_eq!(state.tracks.len(), 2);
        assert_eq!(state.tracks[0].fx.len(), 2);
        assert_eq!(state.tracks[0].fx[0].kind, "echo");
        assert!(state.tracks[0].fx[1].enabled());
        // A track that has never had an effect sends no `fx` at all — empty chain, not an error.
        assert!(state.tracks[1].fx.is_empty());
        assert_eq!(state.master_fx.len(), 1);
        assert_eq!(state.master_fx[0].kind, "limit");
    }

    /// The insert splice is real wiring, not bookkeeping: a muting node spliced into the master
    /// insert must silence the mix, and restoring the wire must bring it back. (A muting gain
    /// stands in for a plugin node — the splice cares only that it is an AudioNode.)
    #[test]
    fn the_master_plugin_insert_splices_and_restores() {
        use web_audio_api::node::AudioScheduledSourceNode;
        let render = |mute: bool, restore: bool| -> f32 {
            let ctx = OfflineAudioContext::new(1, 4096, 44100.0);
            let mut mixer = Mixer::new(&ctx);
            mixer.master.connect(&ctx.destination());
            mixer.ensure(&ctx, 1);
            let mut src = ctx.create_buffer_source();
            let mut buf = ctx.create_buffer(1, 512, 44100.0);
            buf.get_channel_data_mut(0).fill(0.5);
            src.set_buffer(buf);
            src.connect(mixer.input(0).unwrap());
            src.start_at(0.0);
            let g = ctx.create_gain();
            if mute {
                g.gain().set_value(0.0);
                mixer.set_master_plugin_insert(Some(&g));
            }
            if restore {
                mixer.set_master_plugin_insert::<GainNode>(None);
            }
            let mut ctx = ctx;
            let out = ctx.start_rendering_sync();
            out.get_channel_data(0).iter().fold(0.0f32, |m, s| m.max(s.abs()))
        };
        assert!(render(false, false) > 0.01, "the mix must pass with a straight wire");
        assert!(render(true, false) < 1e-6, "a muting insert must actually be in the path");
        assert!(render(true, true) > 0.01, "and restoring the wire must bring the mix back");
    }

    /// A CHAIN of inserts is real wiring in series, in order, and an empty chain is the straight
    /// wire. Gains stand in for plugins: two at 0.5 must multiply to 0.25 — which also proves
    /// they are in SERIES, not summed in parallel (parallel would read 1.0).
    #[test]
    fn a_plugin_chain_wires_in_series_and_unwinds() {
        use web_audio_api::node::AudioScheduledSourceNode;
        let render = |gains: &[f32], clear_after: bool| -> f32 {
            let ctx = OfflineAudioContext::new(1, 4096, 44100.0);
            let mut mixer = Mixer::new(&ctx);
            mixer.master.connect(&ctx.destination());
            mixer.ensure(&ctx, 1);
            // A fresh strip's compressor sits at the SPEC defaults (threshold -24 dB) until
            // `apply` runs — nonlinear, which bends the ratios this test is about. The song
            // default (cmp 0) is a compressor that cannot compress.
            let state: MixState = serde_json::from_str(
                r#"{"tracks":[{"vol":0.85}],"master":0.75,"delayTime":0.3}"#,
            )
            .unwrap();
            mixer.apply(&ctx, &state);
            let mut src = ctx.create_buffer_source();
            let mut buf = ctx.create_buffer(1, 512, 44100.0);
            buf.get_channel_data_mut(0).fill(0.5);
            src.set_buffer(buf);
            src.connect(mixer.input(0).unwrap());
            src.start_at(0.0);
            let nodes: Vec<GainNode> = gains
                .iter()
                .map(|g| {
                    let n = ctx.create_gain();
                    n.gain().set_value(*g);
                    n
                })
                .collect();
            let refs: Vec<&GainNode> = nodes.iter().collect();
            mixer.set_plugin_chain(0, &refs);
            if clear_after {
                mixer.set_plugin_chain::<GainNode>(0, &[]);
            }
            let mut ctx = ctx;
            let out = ctx.start_rendering_sync();
            out.get_channel_data(0).iter().fold(0.0f32, |m, s| m.max(s.abs()))
        };
        let dry = render(&[], false);
        assert!(dry > 0.01, "empty chain must be a wire ({dry})");
        let one = render(&[0.5], false);
        let two = render(&[0.5, 0.5], false);
        assert!(
            (one / dry - 0.5).abs() < 0.02,
            "one link must be heard once (ratio {})", one / dry
        );
        assert!(
            (two / dry - 0.25).abs() < 0.02,
            "two links must multiply — series, not parallel (ratio {})", two / dry
        );
        let back = render(&[0.0, 0.0], true);
        assert!(back > 0.01, "clearing the chain must restore the wire ({back})");
    }

    /// Mute and solo, as `apply` reads them off the trackset. This is audio, not bookkeeping:
    /// a constant is played into two strips and what reaches the destination is measured, because
    /// the bug this pins down ("muting a channel with a vst on it does not mute") was precisely a
    /// flag that existed on one side of the wire and nowhere in the graph.
    #[test]
    fn mute_and_solo_gate_the_strip() {
        let render = |tracks_json: &str| -> (f32, f32) {
            let ctx = OfflineAudioContext::new(1, 4096, 44100.0);
            let mut mixer = Mixer::new(&ctx);
            mixer.master.connect(&ctx.destination());
            mixer.ensure(&ctx, 2);
            let state: MixState = serde_json::from_str(&format!(
                r#"{{"tracks":{tracks_json},"master":0.75,"delayTime":0.3}}"#
            ))
            .unwrap();
            mixer.apply(&ctx, &state);
            // One source per strip, on different halves of the render, so each channel's
            // contribution is measurable on its own out of the summed master.
            for (t, at) in [(0usize, 0usize), (1usize, 2048usize)] {
                let mut src = ctx.create_buffer_source();
                let mut buf = ctx.create_buffer(1, 512, 44100.0);
                buf.get_channel_data_mut(0).fill(0.5);
                src.set_buffer(buf);
                src.connect(mixer.input(t).unwrap());
                src.start_at(at as f64 / 44100.0);
            }
            let mut ctx = ctx;
            let out = ctx.start_rendering_sync();
            let d = out.get_channel_data(0);
            let peak = |r: std::ops::Range<usize>| d[r].iter().fold(0.0f32, |m, s| m.max(s.abs()));
            (peak(0..2048), peak(2048..4096))
        };
        let (a, b) = render(r#"[{"vol":0.85},{"vol":0.85}]"#);
        assert!(a > 0.01 && b > 0.01, "both strips must pass with nothing muted ({a}, {b})");
        let (a, b) = render(r#"[{"vol":0.85,"mute":true},{"vol":0.85}]"#);
        assert!(a < 1e-6, "a muted strip must be silent ({a})");
        assert!(b > 0.01, "and its neighbour untouched ({b})");
        let (a, b) = render(r#"[{"vol":0.85,"solo":true},{"vol":0.85}]"#);
        assert!(a > 0.01, "a soloed strip must sound ({a})");
        assert!(b < 1e-6, "and one solo anywhere silences every non-solo ({b})");
        // Un-muting is the same wholesale apply — the gate must come OFF too.
        let (a, _) = render(r#"[{"vol":0.85,"mute":false},{"vol":0.85}]"#);
        assert!(a > 0.01, "an un-muted strip must come back ({a})");
    }

    /// The unported master limiter must come back out of `apply` as a named skip — that is the
    /// honesty contract: what the panel shows and the engine plays may differ, but never silently.
    #[test]
    fn apply_names_what_it_cannot_build() {
        let ctx = OfflineAudioContext::new(2, 128, 44100.0);
        let mut mixer = Mixer::new(&ctx);
        let state: MixState = serde_json::from_str(
            r#"{"tracks":[{"vol":0.8,"fx":[{"type":"echo","p":{}}]}],
                "masterFx":[{"type":"limit","p":{}}],
                "master":0.75,"delayTime":0.3}"#,
        )
        .unwrap();
        let skipped = mixer.apply(&ctx, &state);
        assert_eq!(skipped, vec!["limit".to_string()]);
        // And the supported chain went in: applying the same state again is a tune, not a rebuild,
        // and still succeeds.
        let again = mixer.apply(&ctx, &state);
        assert_eq!(again, vec!["limit".to_string()]);
    }

    /// Where does a delay actually put its repeats?
    ///
    /// Written to answer one question the master-bus null test raised and could not settle from the
    /// outside: `master-delay` differed from the browser in *shape* rather than level, and the
    /// ping-pong bus's feedback loop was the only thing in it Chrome and this crate might implement
    /// differently. A delay inside a cycle is a special case in the spec — the minimum delay is one
    /// render quantum — so the worry was that the crate charges that quantum on top of the
    /// requested time, which would put every repeat 2.9 ms late.
    ///
    /// Renders an impulse through a delay with feedback and reports where the peaks land.
    fn repeat_positions(delay_secs: f64, feedback: f32) -> Vec<usize> {
        const SR: f32 = 44100.0;
        let len = (SR as usize) / 2;
        let ctx = OfflineAudioContext::new(1, len, SR);

        let mut impulse = ctx.create_buffer(1, 1, SR);
        impulse.get_channel_data_mut(0)[0] = 1.0;
        let mut src = ctx.create_buffer_source();
        src.set_buffer(impulse);

        let delay = ctx.create_delay(2.0);
        delay.delay_time().set_value(delay_secs as f32);
        let fb = ctx.create_gain();
        fb.gain().set_value(feedback);

        src.connect(&delay);
        delay.connect(&fb);
        fb.connect(&delay); // the cycle
        delay.connect(&ctx.destination());
        src.start_at(0.0);

        let mut ctx = ctx;
        let out = ctx.start_rendering_sync();
        let data = out.get_channel_data(0);
        let mut peaks = Vec::new();
        for (i, &s) in data.iter().enumerate() {
            if s.abs() > 0.05 {
                // one index per burst, not one per sample
                if peaks.last().map(|&p: &usize| i - p > 16).unwrap_or(true) {
                    peaks.push(i);
                }
            }
        }
        peaks
    }

    #[test]
    fn a_delay_in_a_cycle_repeats_where_it_was_told_to() {
        let want = 0.1_f64;
        let peaks = repeat_positions(want, 0.5);
        assert!(peaks.len() >= 2, "expected repeats, got {peaks:?}");
        let sr = 44100.0_f64;
        let first = peaks[0] as f64 / sr;
        let gap = (peaks[1] - peaks[0]) as f64 / sr;
        // One render quantum is 2.9 ms at 44.1 kHz. Half of that is a tight enough bound to catch
        // a quantum being charged on top, and loose enough to survive interpolation.
        let tol = 64.0 / sr;
        assert!(
            (first - want).abs() < tol,
            "first repeat at {first:.5}s, asked for {want}s — off by {:.5}s ({:.0} samples)",
            first - want,
            (first - want) * sr
        );
        assert!(
            (gap - want).abs() < tol,
            "repeats {gap:.5}s apart, asked for {want}s — off by {:.0} samples",
            (gap - want) * sr
        );
    }
}

#[cfg(test)]
mod cost {
    use super::*;
    use web_audio_api::context::OfflineAudioContext;

    /// How long does the mixer cost to build and to drive?
    ///
    /// Written the moment the desktop app went unresponsive after the mixer landed. The audio
    /// thread builds a `Master` (which includes a 1.9-second stereo convolver impulse) when a device
    /// opens, and applies a whole `MixState` on every mixer edit — up to 25 times a second while a
    /// fader is dragged. If either of those is expensive, the symptom is exactly "the DAW is not
    /// responding", so it is worth a number rather than an assumption.
    #[test]
    fn building_and_driving_the_mixer_is_cheap() {
        let ctx = OfflineAudioContext::new(2, 44100, 44100.0);

        let t0 = std::time::Instant::now();
        let mut mixer = Mixer::new(&ctx);
        let build = t0.elapsed();

        let state = MixState {
            tracks: vec![TrackSet::default(); 32],
            master: 0.75,
            delay_time: 0.35,
            ..Default::default()
        };

        let t1 = std::time::Instant::now();
        mixer.apply(&ctx, &state); // the first one also builds 32 strips
        let first = t1.elapsed();

        let t2 = std::time::Instant::now();
        for _ in 0..100 {
            mixer.apply(&ctx, &state);
        }
        let per = t2.elapsed() / 100;

        println!("  Master::new            {build:?}");
        println!("  first apply (32 chans) {first:?}");
        println!("  steady apply           {per:?}");

        // A mixer edit arrives up to 25 times a second and shares the audio thread with the note
        // scheduler. Ten milliseconds is already the tick interval, so anything near it is a bug.
        assert!(per.as_millis() < 10, "a mixer update costs {per:?} — that is a stall, not an edit");
        assert!(build.as_millis() < 2000, "building the mixer costs {build:?}");
    }

    /// **EVERY channel reaches the master, and the master's plugins process every channel.**
    ///
    /// The property a mixing desk is expected to have and the one Mickey asked to have proven:
    /// *"make sure that all the channels going through the master channel eventually like a
    /// normal DAW"* (2026-08-15), after a channel with no plugins of its own looked like it was
    /// bypassing the master's. Proven by RENDERING, per the house rule, not by reading the
    /// wiring: a muting node in the MASTER's plugin chain must silence a signal injected into
    /// any channel, one channel at a time, including channels whose own insert chain is empty
    /// while a neighbour's is full.
    #[test]
    fn every_channel_goes_through_the_master_and_its_plugins() {
        use web_audio_api::node::AudioScheduledSourceNode;
        const N: usize = 6;
        // `track` carries the signal; `mute_master` puts a plugin in the MASTER chain.
        let render = |track: usize, mute_master: bool| -> f32 {
            let ctx = OfflineAudioContext::new(1, 4096, 44100.0);
            let mut mixer = Mixer::new(&ctx);
            mixer.master.connect(&ctx.destination());
            mixer.ensure(&ctx, N);
            // Song defaults, so the strips' compressors cannot compress and bend the levels.
            let state: MixState = serde_json::from_str(
                r#"{"tracks":[{"vol":0.85},{"vol":0.85},{"vol":0.85},{"vol":0.85},{"vol":0.85},
                              {"vol":0.85}],"master":0.75,"delayTime":0.3}"#,
            )
            .unwrap();
            mixer.apply(&ctx, &state);
            // ASYMMETRY, on purpose: one OTHER channel owns a plugin chain. This is the shape of
            // the real session — a couple of channels with plugins, the rest bare — and it is
            // what makes "the bare ones are being left out" a question worth asking.
            let neighbour = ctx.create_gain();
            mixer.set_plugin_chain(2, &[&neighbour]);
            let mut src = ctx.create_buffer_source();
            let mut buf = ctx.create_buffer(1, 1024, 44100.0);
            buf.get_channel_data_mut(0).fill(0.5);
            src.set_buffer(buf);
            src.connect(mixer.input(track).expect("every channel must have a strip"));
            src.start_at(0.0);
            let killer = ctx.create_gain();
            if mute_master {
                killer.gain().set_value(0.0);
                mixer.set_master_plugin_chain(&[&killer]);
            }
            // **The mix snapshot arrives AFTER the plugins are spliced, exactly as it does live**
            // — nengMix fires on every fader move and every edit. If `apply` rebuilt the master
            // FX in a way that dropped the plugin splice, the master's plugins would quietly
            // stop processing the moment anything was touched, which is precisely the symptom
            // this test exists to rule out.
            mixer.apply(&ctx, &state);
            let mut ctx = ctx;
            let out = ctx.start_rendering_sync();
            out.get_channel_data(0).iter().fold(0.0f32, |m, s| m.max(s.abs()))
        };
        for t in 0..N {
            let dry = render(t, false);
            let muted = render(t, true);
            assert!(dry > 0.01, "channel {t} never reached the master (peak {dry})");
            assert!(
                muted < 1e-6,
                "channel {t} BYPASSES the master's plugin chain — a muting plugin there left it \
                 at peak {muted}. Every channel must pass through the master's inserts."
            );
        }
    }
}
