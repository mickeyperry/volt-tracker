//! The insert FX — `FX_TYPES` from beta2.html, in Rust.
//!
//! Same discipline as the strip: **a transliteration, not a rewrite.** Every builder below is a
//! node-for-node copy of the matching `build:` in `FX_TYPES` (beta2.html:1196), and every `tune`
//! branch has a twin here in [`Fx::tune`]. The numbers are the browser's numbers; where the browser
//! computes `1 + drive * 30`, so does this.
//!
//! Why this exists: the channel strip's insert slot (`fx_in`/`fx_out` in `mixer.rs`) has been a
//! straight wire since stage A, which meant an effect added in the mixer panel changed nothing a
//! VST played — the plugin's audio lives in this graph, not the browser's. Mickey, 2026-08-12:
//! *"if ill add effects it wont sound different"*. He was right; this is the fix.
//!
//! **Two of the fifteen are not here, on purpose.**
//!   * `limit` (Limiter L∞) is a hand-written look-ahead brickwall in a ScriptProcessor — a real
//!     DSP kernel, not a node graph, and porting it is its own job with its own null test.
//!   * `pitch` is the delay-ramp Doppler shifter; portable, but its buffer-driven ramp machinery
//!     is fiddly enough to be a separate careful port.
//! An unported effect in a chain is SKIPPED (the chain stays connected around it) and reported
//! once through `neng:err`, so the difference is stated rather than silent.
//!
//! Rebuild policy, because `eng_mix` arrives wholesale up to 25 times a second during a fader
//! drag: a chain is torn down and rebuilt only when its *shape* changes (types added, removed,
//! reordered, bypassed). A parameter change tunes the existing nodes, exactly as the browser
//! splits `buildFx` from `tune`. The expensive insides — waveshaper curves, the reverb impulse —
//! rebuild only when the parameter that shapes them actually moved.

use std::collections::HashMap;

use serde::Deserialize;
use web_audio_api::context::BaseAudioContext;
use web_audio_api::node::{
    AudioNode, AudioScheduledSourceNode, BiquadFilterNode, BiquadFilterType, ChannelMergerNode,
    ConvolverNode, DelayNode, DynamicsCompressorNode, GainNode, OscillatorNode, OverSampleType,
    StereoPannerNode, WaveShaperNode,
};

use crate::seed_rnd;

/// One effect as it sits in the song file — `song.trackset[t].fx[i]` / `song.masterFx[i]`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FxSet {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub p: HashMap<String, f32>,
    /// `fx.on === false` is bypass; anything else (including absent) is on. The browser writes
    /// `on:false` on bypass and deletes or omits it otherwise.
    #[serde(default)]
    pub on: Option<bool>,
}

impl FxSet {
    pub fn enabled(&self) -> bool {
        self.on != Some(false)
    }
    /// A parameter, or the browser's default for this effect type — `ft.p[k][2]` in `buildFx`.
    fn p(&self, k: &str) -> f32 {
        self.p.get(k).copied().unwrap_or_else(|| default_of(&self.kind, k))
    }
}

/// `ft.p[k][2]` — the defaults column of `FX_TYPES`, verbatim.
fn default_of(kind: &str, k: &str) -> f32 {
    match (kind, k) {
        ("verb", "size") => 2.2, ("verb", "damp") => 0.45, ("verb", "pre") => 0.02, ("verb", "mix") => 0.3,
        ("echo", "time") => 0.28, ("echo", "fb") => 0.42, ("echo", "tone") => 4200.0, ("echo", "width") => 1.0, ("echo", "mix") => 0.3,
        ("comp", "thresh") => -24.0, ("comp", "ratio") => 4.0, ("comp", "att") => 0.01, ("comp", "rel") => 0.18, ("comp", "makeup") => 6.0, ("comp", "mix") => 1.0,
        ("crush", "bits") => 8.0, ("crush", "mix") => 1.0,
        ("pulteq", "lowHz") => 60.0, ("pulteq", "lowUp") => 0.0, ("pulteq", "lowDown") => 0.0, ("pulteq", "airHz") => 10000.0, ("pulteq", "airUp") => 0.0,
        ("softclip", "drive") => 1.4, ("softclip", "knee") => 0.6, ("softclip", "ceil") => -0.3, ("softclip", "mix") => 1.0,
        ("flanger", "rate") => 0.3, ("flanger", "depth") => 0.7, ("flanger", "fb") => 0.5, ("flanger", "mix") => 0.5,
        ("chorus", "rate") => 0.8, ("chorus", "depth") => 0.5, ("chorus", "mix") => 0.5,
        ("autofilter", "cut") => 0.55, ("autofilter", "res") => 8.0, ("autofilter", "rate") => 0.5, ("autofilter", "amt") => 0.5, ("autofilter", "mode") => 0.0,
        ("phaser", "rate") => 0.5, ("phaser", "depth") => 0.7, ("phaser", "mix") => 0.6,
        ("widen", "width") => 0.5, ("widen", "mix") => 1.0,
        ("superdrive", "drive") => 0.7, ("superdrive", "tone") => 3800.0, ("superdrive", "level") => 0.5, ("superdrive", "mix") => 1.0,
        ("sausage", "fat") => 0.5, ("sausage", "color") => 0.3, ("sausage", "pump") => 0.4, ("sausage", "level") => 0.6,
        _ => 0.0,
    }
}

/// Is this a type the native chain can build? The two absentees are documented in the header.
pub fn supported(kind: &str) -> bool {
    matches!(
        kind,
        "verb" | "echo" | "comp" | "crush" | "pulteq" | "softclip" | "flanger" | "chorus"
            | "autofilter" | "phaser" | "widen" | "superdrive" | "sausage"
    )
}

// ---- the transfer curves, verbatim from beta2.html ---------------------------------------------

/// `crushCurve(bits)` — a staircase with 2^bits steps.
fn crush_curve(bits: f32) -> Vec<f32> {
    let steps = 2f32.powf(bits.clamp(1.0, 16.0));
    (0..2048)
        .map(|i| {
            let x = i as f32 / 2047.0 * 2.0 - 1.0;
            (x * steps).round() / steps
        })
        .collect()
}

/// `softClipCurve(knee)` — linear to the threshold, tanh-rounded above it.
fn soft_clip_curve(knee: f32) -> Vec<f32> {
    let th = 1.0 - (knee * 0.9).min(0.98);
    (0..2048)
        .map(|i| {
            let x = i as f32 / 2047.0 * 2.0 - 1.0;
            let a = x.abs();
            let s = if x < 0.0 { -1.0 } else { 1.0 };
            if a <= th { x } else { s * (th + (1.0 - th) * ((a - th) / (1.0 - th)).tanh()) }
        })
        .collect()
}

/// `mkIR(sec, damp)` — the insert reverb's impulse: stereo decorrelated noise, exponential tail,
/// damping that deepens along it, energy-normalised. The browser rolls unseeded dice here, so
/// there is no bit-identical target; the seeded generator gives a tail with the same statistics
/// and makes native renders reproducible, which the browser's never were.
fn mk_ir<C: BaseAudioContext>(ctx: &C, sec: f32, damp: f32) -> web_audio_api::AudioBuffer {
    let sr = ctx.sample_rate();
    let n = ((sr * 0.05) as usize).max((sr * sec) as usize);
    let mut buf = ctx.create_buffer(2, n, sr);
    let mut rnd = seed_rnd(0x1AB0DE);
    for ch in 0..2 {
        let data = buf.get_channel_data_mut(ch);
        let mut lp = 0.0f64;
        let mut sum = 0.0f64;
        for (i, s) in data.iter_mut().enumerate() {
            let t = i as f64 / n as f64;
            let a = (1.0 - damp as f64 * (0.25 + 0.72 * t)).max(0.03);
            lp += ((rnd() * 2.0 - 1.0) - lp) * a;
            let v = lp * (-6.9 * t).exp();
            *s = v as f32;
            sum += v * v;
        }
        let g = (1.6 / sum.max(1e-9)).sqrt() as f32;
        for s in data.iter_mut() {
            *s *= g;
        }
    }
    buf
}

/// `driveCurveArr(amt)` lives in lib.rs as `drive_curve`; re-exported here for the two drives.
use crate::drive_curve;

// ---- one built effect --------------------------------------------------------------------------

/// A built effect: its two ends, its inner nodes, and the state its `tune` needs.
///
/// One enum rather than a trait object so the whole chain lives in a `Vec` with no boxing and the
/// borrow checker can see every field — these run on the audio thread's control side and nothing
/// here is allowed to allocate per block (it only allocates on build and on the guarded rebuilds).
pub enum Fx {
    Verb {
        input: GainNode, out: GainNode,
        pre: DelayNode, cv: ConvolverNode, dry: GainNode, wet: GainNode,
        _hp: BiquadFilterNode,
        size: f32, damp: f32,
    },
    Echo {
        input: GainNode, out: GainNode,
        dl_l: DelayNode, dl_r: DelayNode, fb_l: GainNode, fb_r: GainNode,
        lp_l: BiquadFilterNode, lp_r: BiquadFilterNode,
        pan_l: StereoPannerNode, pan_r: StereoPannerNode,
        dry: GainNode, wet: GainNode,
        _hp: Vec<BiquadFilterNode>,
    },
    Comp {
        input: GainNode, out: GainNode,
        cmp: DynamicsCompressorNode, mk: GainNode, dry: GainNode, wet: GainNode,
    },
    Crush {
        input: GainNode, out: GainNode,
        ws: WaveShaperNode, dry: GainNode, wet: GainNode,
        bits: f32,
    },
    Pulteq {
        input: GainNode, out: GainNode,
        up: BiquadFilterNode, dn: BiquadFilterNode, air: BiquadFilterNode,
    },
    SoftClip {
        input: GainNode, out: GainNode,
        pre: GainNode, ws: WaveShaperNode, post: GainNode, dry: GainNode, wet: GainNode,
        knee: f32,
    },
    Flanger {
        input: GainNode, out: GainNode,
        lfo: OscillatorNode, lg: GainNode, fb: GainNode, dry: GainNode, wet: GainNode,
        _dl: DelayNode,
    },
    Chorus {
        input: GainNode, out: GainNode,
        lfo: OscillatorNode, lg: GainNode, dry: GainNode, wet: GainNode,
        _dl: DelayNode,
    },
    AutoFilter {
        input: GainNode, out: GainNode,
        f1: BiquadFilterNode, f2: BiquadFilterNode, lfo: OscillatorNode, lg: GainNode,
    },
    Phaser {
        input: GainNode, out: GainNode,
        lfo: OscillatorNode, lg: GainNode, dry: GainNode, wet: GainNode,
        _aps: Vec<BiquadFilterNode>,
    },
    Widen {
        input: GainNode, out: GainNode,
        dl: DelayNode, dry: GainNode, wet: GainNode,
        _mg: ChannelMergerNode,
    },
    SuperDrive {
        input: GainNode, out: GainNode,
        pre: GainNode, ws: WaveShaperNode, lp: BiquadFilterNode, lvl: GainNode,
        dry: GainNode, wet: GainNode,
        drive: f32,
    },
    Sausage {
        input: GainNode, out: GainNode,
        pre: GainNode, ws: WaveShaperNode, tilt: BiquadFilterNode, air: BiquadFilterNode,
        cmp: DynamicsCompressorNode, lvl: GainNode,
        fat: f32,
    },
}

impl Fx {
    /// `FX_TYPES[kind].build(p)`. `None` for a type this port does not have.
    pub fn build<C: BaseAudioContext>(ctx: &C, set: &FxSet) -> Option<Fx> {
        let p = |k: &str| set.p(k);
        Some(match set.kind.as_str() {
            "verb" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let pre = ctx.create_delay(0.15);
                pre.delay_time().set_value(p("pre"));
                let mut cv = ctx.create_convolver();
                cv.set_buffer(mk_ir(ctx, p("size"), p("damp")));
                let mut hp = ctx.create_biquad_filter();
                hp.set_type(BiquadFilterType::Highpass);
                hp.frequency().set_value(110.0);
                hp.q().set_value(0.5);
                dry.gain().set_value(1.0 - p("mix") * 0.6);
                wet.gain().set_value(p("mix"));
                input.connect(&pre);
                pre.connect(&cv);
                cv.connect(&hp);
                hp.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                Fx::Verb { input, out, pre, cv, dry, wet, _hp: hp, size: p("size"), damp: p("damp") }
            }
            "echo" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let dl_l = ctx.create_delay(2.0);
                let dl_r = ctx.create_delay(2.0);
                dl_l.delay_time().set_value(p("time"));
                dl_r.delay_time().set_value(p("time"));
                let fb_l = ctx.create_gain();
                let fb_r = ctx.create_gain();
                fb_l.gain().set_value(p("fb"));
                fb_r.gain().set_value(p("fb"));
                let mut lp_l = ctx.create_biquad_filter();
                let mut lp_r = ctx.create_biquad_filter();
                lp_l.set_type(BiquadFilterType::Lowpass);
                lp_r.set_type(BiquadFilterType::Lowpass);
                lp_l.frequency().set_value(p("tone"));
                lp_r.frequency().set_value(p("tone"));
                let mut hp_l = ctx.create_biquad_filter();
                let mut hp_r = ctx.create_biquad_filter();
                hp_l.set_type(BiquadFilterType::Highpass);
                hp_r.set_type(BiquadFilterType::Highpass);
                hp_l.frequency().set_value(130.0);
                hp_r.frequency().set_value(130.0);
                let pan_l = ctx.create_stereo_panner();
                let pan_r = ctx.create_stereo_panner();
                pan_l.pan().set_value(-p("width"));
                pan_r.pan().set_value(p("width"));
                input.connect(&dl_l);
                dl_l.connect(&lp_l);
                lp_l.connect(&hp_l);
                hp_l.connect(&fb_l);
                fb_l.connect(&dl_r); // L bounces to R
                dl_r.connect(&lp_r);
                lp_r.connect(&hp_r);
                hp_r.connect(&fb_r);
                fb_r.connect(&dl_l); // and back
                dl_l.connect(&pan_l);
                pan_l.connect(&wet);
                dl_r.connect(&pan_r);
                pan_r.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                dry.gain().set_value(1.0 - p("mix") * 0.5);
                wet.gain().set_value(p("mix"));
                Fx::Echo { input, out, dl_l, dl_r, fb_l, fb_r, lp_l, lp_r, pan_l, pan_r, dry, wet, _hp: vec![hp_l, hp_r] }
            }
            "comp" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let cmp = ctx.create_dynamics_compressor();
                cmp.threshold().set_value(p("thresh"));
                cmp.ratio().set_value(p("ratio"));
                cmp.knee().set_value(6.0);
                cmp.attack().set_value(p("att"));
                cmp.release().set_value(p("rel"));
                let mk = ctx.create_gain();
                mk.gain().set_value(10f32.powf(p("makeup") / 20.0));
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
                input.connect(&cmp);
                cmp.connect(&mk);
                mk.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                Fx::Comp { input, out, cmp, mk, dry, wet }
            }
            "crush" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let mut ws = ctx.create_wave_shaper();
                ws.set_curve(crush_curve(p("bits")));
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
                input.connect(&ws);
                ws.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                Fx::Crush { input, out, ws, dry, wet, bits: p("bits") }
            }
            "pulteq" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let mut up = ctx.create_biquad_filter();
                up.set_type(BiquadFilterType::Lowshelf);
                up.frequency().set_value(p("lowHz"));
                up.gain().set_value(p("lowUp"));
                let mut dn = ctx.create_biquad_filter();
                dn.set_type(BiquadFilterType::Peaking);
                dn.frequency().set_value(p("lowHz") * 2.2);
                dn.q().set_value(0.8);
                dn.gain().set_value(-p("lowDown"));
                let mut air = ctx.create_biquad_filter();
                air.set_type(BiquadFilterType::Peaking);
                air.frequency().set_value(p("airHz"));
                air.q().set_value(0.7);
                air.gain().set_value(p("airUp"));
                input.connect(&up);
                up.connect(&dn);
                dn.connect(&air);
                air.connect(&out);
                Fx::Pulteq { input, out, up, dn, air }
            }
            "softclip" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let pre = ctx.create_gain();
                pre.gain().set_value(p("drive"));
                let mut ws = ctx.create_wave_shaper();
                ws.set_curve(soft_clip_curve(p("knee")));
                ws.set_oversample(OverSampleType::X4);
                let post = ctx.create_gain();
                post.gain()
                    .set_value(10f32.powf(p("ceil") / 20.0) / p("drive").sqrt().max(1.0));
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
                input.connect(&pre);
                pre.connect(&ws);
                ws.connect(&post);
                post.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                Fx::SoftClip { input, out, pre, ws, post, dry, wet, knee: p("knee") }
            }
            "flanger" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let dl = ctx.create_delay(0.05);
                dl.delay_time().set_value(0.004);
                let mut lfo = ctx.create_oscillator();
                lfo.frequency().set_value(p("rate"));
                let lg = ctx.create_gain();
                lg.gain().set_value(0.0035 * p("depth"));
                lfo.connect(&lg);
                lg.connect(dl.delay_time());
                lfo.start();
                let fb = ctx.create_gain();
                fb.gain().set_value(p("fb"));
                dl.connect(&fb);
                fb.connect(&dl);
                dry.gain().set_value(1.0 - p("mix") * 0.5);
                wet.gain().set_value(p("mix"));
                input.connect(&dl);
                dl.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                Fx::Flanger { input, out, lfo, lg, fb, dry, wet, _dl: dl }
            }
            "chorus" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let dl = ctx.create_delay(0.08);
                dl.delay_time().set_value(0.018);
                let mut lfo = ctx.create_oscillator();
                lfo.frequency().set_value(p("rate"));
                let lg = ctx.create_gain();
                lg.gain().set_value(0.008 * p("depth"));
                lfo.connect(&lg);
                lg.connect(dl.delay_time());
                lfo.start();
                dry.gain().set_value(1.0 - p("mix") * 0.4);
                wet.gain().set_value(p("mix"));
                input.connect(&dl);
                dl.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                Fx::Chorus { input, out, lfo, lg, dry, wet, _dl: dl }
            }
            "autofilter" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let mode = p("mode");
                let ty = af_type(mode);
                let fz = 40.0 * 300f32.powf(p("cut"));
                let mut f1 = ctx.create_biquad_filter();
                let mut f2 = ctx.create_biquad_filter();
                f1.set_type(ty);
                f2.set_type(ty);
                f1.frequency().set_value(fz);
                f2.frequency().set_value(fz);
                f1.q().set_value(p("res"));
                f2.q().set_value((p("res") * 0.3).max(0.5));
                let mut lfo = ctx.create_oscillator();
                lfo.frequency().set_value(p("rate"));
                let lg = ctx.create_gain();
                lg.gain().set_value(p("amt") * fz * 0.9);
                lfo.connect(&lg);
                lg.connect(f1.frequency());
                lg.connect(f2.frequency());
                lfo.start();
                input.connect(&f1);
                f1.connect(&f2);
                f2.connect(&out);
                Fx::AutoFilter { input, out, f1, f2, lfo, lg }
            }
            "phaser" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let mut lfo = ctx.create_oscillator();
                lfo.frequency().set_value(p("rate"));
                let lg = ctx.create_gain();
                lg.gain().set_value(900.0 * p("depth"));
                lfo.connect(&lg);
                lfo.start();
                let mut aps = Vec::with_capacity(4);
                for f in [400.0f32, 800.0, 1600.0, 2400.0] {
                    let mut ap = ctx.create_biquad_filter();
                    ap.set_type(BiquadFilterType::Allpass);
                    ap.frequency().set_value(f);
                    ap.q().set_value(0.6);
                    lg.connect(ap.frequency());
                    match aps.last() {
                        Some(prev) => {
                            let prev: &BiquadFilterNode = prev;
                            prev.connect(&ap);
                        }
                        None => {
                            input.connect(&ap);
                        }
                    }
                    aps.push(ap);
                }
                dry.gain().set_value(1.0 - p("mix") * 0.5);
                wet.gain().set_value(p("mix"));
                aps.last().unwrap().connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                Fx::Phaser { input, out, lfo, lg, dry, wet, _aps: aps }
            }
            "widen" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let mg = ctx.create_channel_merger(2);
                let dl = ctx.create_delay(0.05);
                dl.delay_time().set_value(0.001 + 0.016 * p("width"));
                input.connect_from_output_to_input(&mg, 0, 0);
                input.connect(&dl);
                dl.connect_from_output_to_input(&mg, 0, 1);
                mg.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
                Fx::Widen { input, out, dl, dry, wet, _mg: mg }
            }
            "superdrive" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let dry = ctx.create_gain();
                let wet = ctx.create_gain();
                let pre = ctx.create_gain();
                pre.gain().set_value(1.0 + p("drive") * 30.0);
                let mut ws = ctx.create_wave_shaper();
                ws.set_curve(drive_curve(0.4 + p("drive") * 0.6));
                ws.set_oversample(OverSampleType::X4);
                let mut lp = ctx.create_biquad_filter();
                lp.set_type(BiquadFilterType::Lowpass);
                lp.frequency().set_value(p("tone"));
                let lvl = ctx.create_gain();
                lvl.gain().set_value(p("level"));
                input.connect(&pre);
                pre.connect(&ws);
                ws.connect(&lp);
                lp.connect(&lvl);
                lvl.connect(&wet);
                wet.connect(&out);
                input.connect(&dry);
                dry.connect(&out);
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
                Fx::SuperDrive { input, out, pre, ws, lp, lvl, dry, wet, drive: p("drive") }
            }
            "sausage" => {
                let input = ctx.create_gain();
                let out = ctx.create_gain();
                let pre = ctx.create_gain();
                pre.gain().set_value(1.0 + p("fat") * 8.0);
                let mut ws = ctx.create_wave_shaper();
                ws.set_curve(drive_curve(0.2 + p("fat") * 0.5));
                ws.set_oversample(OverSampleType::X2);
                let mut tilt = ctx.create_biquad_filter();
                tilt.set_type(BiquadFilterType::Lowshelf);
                tilt.frequency().set_value(300.0);
                tilt.gain().set_value(p("color") * 8.0);
                let mut air = ctx.create_biquad_filter();
                air.set_type(BiquadFilterType::Highshelf);
                air.frequency().set_value(8000.0);
                air.gain().set_value(p("color") * 4.0);
                let cmp = ctx.create_dynamics_compressor();
                cmp.threshold().set_value(-24.0 * p("pump") - 6.0);
                cmp.knee().set_value(6.0);
                cmp.ratio().set_value(4.0 + p("pump") * 8.0);
                cmp.attack().set_value(0.004);
                cmp.release().set_value(0.12);
                let lvl = ctx.create_gain();
                lvl.gain().set_value(p("level") * 1.6);
                input.connect(&pre);
                pre.connect(&ws);
                ws.connect(&tilt);
                tilt.connect(&air);
                air.connect(&cmp);
                cmp.connect(&lvl);
                lvl.connect(&out);
                Fx::Sausage { input, out, pre, ws, tilt, air, cmp, lvl, fat: p("fat") }
            }
            _ => return None,
        })
    }

    /// The effect's two ends, for chaining.
    pub fn ends(&self) -> (&GainNode, &GainNode) {
        match self {
            Fx::Verb { input, out, .. }
            | Fx::Echo { input, out, .. }
            | Fx::Comp { input, out, .. }
            | Fx::Crush { input, out, .. }
            | Fx::Pulteq { input, out, .. }
            | Fx::SoftClip { input, out, .. }
            | Fx::Flanger { input, out, .. }
            | Fx::Chorus { input, out, .. }
            | Fx::AutoFilter { input, out, .. }
            | Fx::Phaser { input, out, .. }
            | Fx::Widen { input, out, .. }
            | Fx::SuperDrive { input, out, .. }
            | Fx::Sausage { input, out, .. } => (input, out),
        }
    }

    /// `FX_TYPES[kind].tune`, applied wholesale — every parameter, every time, exactly as the strip
    /// re-applies itself. The guarded pieces (curves, the reverb impulse) rebuild only when the
    /// parameter that shapes them moved, because those are the expensive ones.
    ///
    /// Returns `false` when the change needs a full rebuild (the reverb's impulse — the crate's
    /// convolver takes one buffer for its lifetime), and the caller rebuilds this one effect.
    pub fn tune(&mut self, set: &FxSet) -> bool {
        let p = |k: &str| set.p(k);
        match self {
            Fx::Verb { pre, dry, wet, size, damp, .. } => {
                if (p("size") - *size).abs() > 1e-6 || (p("damp") - *damp).abs() > 1e-6 {
                    return false; // new impulse → rebuild me
                }
                pre.delay_time().set_value(p("pre"));
                dry.gain().set_value(1.0 - p("mix") * 0.6);
                wet.gain().set_value(p("mix"));
            }
            Fx::Echo { dl_l, dl_r, fb_l, fb_r, lp_l, lp_r, pan_l, pan_r, dry, wet, .. } => {
                dl_l.delay_time().set_value(p("time"));
                dl_r.delay_time().set_value(p("time"));
                fb_l.gain().set_value(p("fb"));
                fb_r.gain().set_value(p("fb"));
                lp_l.frequency().set_value(p("tone"));
                lp_r.frequency().set_value(p("tone"));
                pan_l.pan().set_value(-p("width"));
                pan_r.pan().set_value(p("width"));
                dry.gain().set_value(1.0 - p("mix") * 0.5);
                wet.gain().set_value(p("mix"));
            }
            Fx::Comp { cmp, mk, dry, wet, .. } => {
                cmp.threshold().set_value(p("thresh"));
                cmp.ratio().set_value(p("ratio"));
                cmp.attack().set_value(p("att"));
                cmp.release().set_value(p("rel"));
                mk.gain().set_value(10f32.powf(p("makeup") / 20.0));
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
            }
            Fx::Crush { ws, dry, wet, bits, .. } => {
                if (p("bits") - *bits).abs() > 1e-6 {
                    *bits = p("bits");
                    ws.set_curve(crush_curve(*bits));
                }
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
            }
            Fx::Pulteq { up, dn, air, .. } => {
                up.frequency().set_value(p("lowHz"));
                dn.frequency().set_value(p("lowHz") * 2.2);
                up.gain().set_value(p("lowUp"));
                dn.gain().set_value(-p("lowDown"));
                air.frequency().set_value(p("airHz"));
                air.gain().set_value(p("airUp"));
            }
            Fx::SoftClip { pre, ws, post, dry, wet, knee, .. } => {
                if (p("knee") - *knee).abs() > 1e-6 {
                    *knee = p("knee");
                    ws.set_curve(soft_clip_curve(*knee));
                }
                pre.gain().set_value(p("drive"));
                post.gain()
                    .set_value(10f32.powf(p("ceil") / 20.0) / p("drive").sqrt().max(1.0));
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
            }
            Fx::Flanger { lfo, lg, fb, dry, wet, .. } => {
                lfo.frequency().set_value(p("rate"));
                lg.gain().set_value(0.0035 * p("depth"));
                fb.gain().set_value(p("fb"));
                dry.gain().set_value(1.0 - p("mix") * 0.5);
                wet.gain().set_value(p("mix"));
            }
            Fx::Chorus { lfo, lg, dry, wet, .. } => {
                lfo.frequency().set_value(p("rate"));
                lg.gain().set_value(0.008 * p("depth"));
                dry.gain().set_value(1.0 - p("mix") * 0.4);
                wet.gain().set_value(p("mix"));
            }
            Fx::AutoFilter { f1, f2, lfo, lg, .. } => {
                let ty = af_type(p("mode"));
                f1.set_type(ty);
                f2.set_type(ty);
                let fz = 40.0 * 300f32.powf(p("cut"));
                f1.frequency().set_value(fz);
                f2.frequency().set_value(fz);
                f1.q().set_value(p("res"));
                f2.q().set_value((p("res") * 0.3).max(0.5));
                lfo.frequency().set_value(p("rate"));
                lg.gain().set_value(p("amt") * fz * 0.9);
            }
            Fx::Phaser { lfo, lg, dry, wet, .. } => {
                lfo.frequency().set_value(p("rate"));
                lg.gain().set_value(900.0 * p("depth"));
                dry.gain().set_value(1.0 - p("mix") * 0.5);
                wet.gain().set_value(p("mix"));
            }
            Fx::Widen { dl, dry, wet, .. } => {
                dl.delay_time().set_value(0.001 + 0.016 * p("width"));
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
            }
            Fx::SuperDrive { pre, ws, lp, lvl, dry, wet, drive, .. } => {
                if (p("drive") - *drive).abs() > 1e-6 {
                    *drive = p("drive");
                    ws.set_curve(drive_curve(0.4 + *drive * 0.6));
                }
                pre.gain().set_value(1.0 + p("drive") * 30.0);
                lp.frequency().set_value(p("tone"));
                lvl.gain().set_value(p("level"));
                dry.gain().set_value(1.0 - p("mix"));
                wet.gain().set_value(p("mix"));
            }
            Fx::Sausage { pre, ws, tilt, air, cmp, lvl, fat, .. } => {
                if (p("fat") - *fat).abs() > 1e-6 {
                    *fat = p("fat");
                    ws.set_curve(drive_curve(0.2 + *fat * 0.5));
                }
                pre.gain().set_value(1.0 + p("fat") * 8.0);
                tilt.gain().set_value(p("color") * 8.0);
                air.gain().set_value(p("color") * 4.0);
                cmp.threshold().set_value(-24.0 * p("pump") - 6.0);
                cmp.ratio().set_value(4.0 + p("pump") * 8.0);
                lvl.gain().set_value(p("level") * 1.6);
            }
        }
        true
    }
}

fn af_type(mode: f32) -> BiquadFilterType {
    if mode >= 1.5 {
        BiquadFilterType::Highpass
    } else if mode >= 0.5 {
        BiquadFilterType::Bandpass
    } else {
        BiquadFilterType::Lowpass
    }
}

/// An insert chain living between two fixed gains — `buildFx(ch, list)` plus its rebuild policy.
///
/// Owns everything between `fx_in` and `fx_out` but not those two nodes themselves; they belong to
/// the strip (or the master) and never change identity, which is what lets the chain be torn down
/// and rebuilt without touching the mixer around it.
#[derive(Default)]
pub struct FxChain {
    built: Vec<(String, Fx)>,
    /// The shape the current build answers to: enabled, supported types in order.
    shape: Vec<String>,
}

impl FxChain {
    /// Bring the chain up to date with `list`. Rebuilds on a shape change, tunes otherwise.
    /// Returns the types it had to skip (unported), each once, for the caller to report.
    pub fn apply<C: BaseAudioContext>(
        &mut self,
        ctx: &C,
        fx_in: &GainNode,
        fx_out: &GainNode,
        list: &[FxSet],
    ) -> Vec<String> {
        let mut skipped = Vec::new();
        let want: Vec<&FxSet> = list
            .iter()
            .filter(|f| {
                if !f.enabled() {
                    return false;
                }
                if !supported(&f.kind) {
                    if !skipped.contains(&f.kind) {
                        skipped.push(f.kind.clone());
                    }
                    return false;
                }
                true
            })
            .collect();
        let shape: Vec<String> = want.iter().map(|f| f.kind.clone()).collect();

        if shape != self.shape {
            self.rebuild(ctx, fx_in, fx_out, &want, shape);
            return skipped;
        }
        // Same shape: tune in place. An effect that cannot be tuned to its new parameters (the
        // reverb's impulse) forces one full rebuild — rare, and exactly what the browser's
        // debounced IR rebuild amounts to.
        let needs_rebuild = self
            .built
            .iter_mut()
            .zip(want.iter())
            .any(|((_, fx), set)| !fx.tune(set));
        if needs_rebuild {
            self.rebuild(ctx, fx_in, fx_out, &want, self.shape.clone());
        }
        skipped
    }

    fn rebuild<C: BaseAudioContext>(
        &mut self,
        ctx: &C,
        fx_in: &GainNode,
        fx_out: &GainNode,
        want: &[&FxSet],
        shape: Vec<String>,
    ) {
        // `ch.fxIn.disconnect()` — every edge out of the head, gone; the old nodes follow when
        // they drop at the end of this function.
        fx_in.disconnect();
        for (_, fx) in self.built.drain(..) {
            let (i, o) = fx.ends();
            i.disconnect();
            o.disconnect();
        }
        let mut new: Vec<(String, Fx)> = Vec::with_capacity(want.len());
        for set in want {
            if let Some(fx) = Fx::build(ctx, set) {
                match new.last() {
                    Some((_, prev)) => prev.ends().1.connect(fx.ends().0),
                    None => fx_in.connect(fx.ends().0),
                };
                new.push((set.kind.clone(), fx));
            }
        }
        match new.last() {
            Some((_, last)) => last.ends().1.connect(fx_out),
            None => fx_in.connect(fx_out),
        };
        self.built = new;
        self.shape = shape;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
    use web_audio_api::node::AudioScheduledSourceNode;

    /// The JS side sends `ts.fx` exactly as it sits in the song file. This is the wire contract:
    /// a field renamed on either side becomes a silently-empty chain, not an error.
    #[test]
    fn a_chain_from_the_song_file_deserialises() {
        let list: Vec<FxSet> = serde_json::from_str(
            r#"[{"type":"echo","p":{"time":0.5,"fb":0.3},"on":true},
                {"type":"verb","p":{}},
                {"type":"limit","p":{"loud":40}},
                {"type":"crush","on":false}]"#,
        )
        .expect("song-file fx must deserialise");
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].kind, "echo");
        assert_eq!(list[0].p("time"), 0.5);
        // A missing parameter reads as the browser's default, not zero.
        assert_eq!(list[0].p("tone"), 4200.0);
        assert_eq!(list[1].p("mix"), 0.3);
        // `on` absent means enabled; `on:false` means bypassed.
        assert!(list[1].enabled());
        assert!(!list[3].enabled());
        // And the two unported types are known to be unported.
        assert!(!supported("limit"));
        assert!(!supported("pitch"));
        assert!(supported("echo") && supported("verb") && supported("crush"));
    }

    fn render_through(list: &[FxSet], len: usize) -> Vec<f32> {
        const SR: f32 = 44100.0;
        let mut ctx = OfflineAudioContext::new(1, len, SR);
        let fx_in = ctx.create_gain();
        let fx_out = ctx.create_gain();
        // The strip wires its insert slot straight through at construction (`fx_in.connect(fx_out)`
        // in Channel::new); the chain only ever REPLACES that wire. An empty or fully-bypassed
        // chain leaving it in place is the contract, so the harness has to start the same way.
        fx_in.connect(&fx_out);
        fx_out.connect(&ctx.destination());
        let mut chain = FxChain::default();
        let skipped = chain.apply(&ctx, &fx_in, &fx_out, list);
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");

        let mut impulse = ctx.create_buffer(1, 1, SR);
        impulse.get_channel_data_mut(0)[0] = 1.0;
        let mut src = ctx.create_buffer_source();
        src.set_buffer(impulse);
        src.connect(&fx_in);
        src.start_at(0.0);

        let out = ctx.start_rendering_sync();
        out.get_channel_data(0).to_vec()
    }

    /// The echo repeats where it was told to — the one timing claim a delay effect makes.
    #[test]
    fn the_echo_repeats_on_time() {
        let list: Vec<FxSet> = serde_json::from_str(
            r#"[{"type":"echo","p":{"time":0.1,"fb":0.5,"mix":1.0,"width":0.0}}]"#,
        )
        .unwrap();
        let data = render_through(&list, 44100 / 2);
        let sr = 44100.0f64;
        // The dry impulse lands at 0; the first repeat must land at 0.1 s ± half a render quantum.
        let first_repeat = data
            .iter()
            .enumerate()
            .skip(800) // past the dry impulse and the filters' ringing
            .find(|(_, s)| s.abs() > 0.05)
            .map(|(i, _)| i as f64 / sr)
            .expect("no repeat found");
        assert!(
            (first_repeat - 0.1).abs() < 64.0 / sr,
            "first repeat at {first_repeat:.4}s, wanted 0.1s"
        );
    }

    /// A bypassed chain must be a straight wire — bit-identical, not merely quiet.
    #[test]
    fn a_bypassed_chain_is_a_wire() {
        let list: Vec<FxSet> =
            serde_json::from_str(r#"[{"type":"crush","on":false},{"type":"verb","on":false}]"#)
                .unwrap();
        let data = render_through(&list, 1024);
        assert_eq!(data[0], 1.0, "the impulse must pass through untouched");
        assert!(data[1..].iter().all(|s| *s == 0.0), "and nothing else may sound");
    }

    /// An unported effect is skipped and NAMED, and the chain around it still works.
    #[test]
    fn an_unported_effect_is_skipped_and_said() {
        let ctx = OfflineAudioContext::new(1, 128, 44100.0);
        let fx_in = ctx.create_gain();
        let fx_out = ctx.create_gain();
        let list: Vec<FxSet> = serde_json::from_str(
            r#"[{"type":"limit","p":{}},{"type":"crush","p":{}},{"type":"pitch","p":{}}]"#,
        )
        .unwrap();
        let mut chain = FxChain::default();
        let skipped = chain.apply(&ctx, &fx_in, &fx_out, &list);
        assert_eq!(skipped, vec!["limit".to_string(), "pitch".to_string()]);
        // And asking again with the same list reports the same skips — the DEDUPE lives at the
        // engine's `said` layer, not here, or a skip would be reported once per session ever.
        let again = chain.apply(&ctx, &fx_in, &fx_out, &list);
        assert_eq!(again, skipped);
    }

    /// Tuning must not rebuild: the same node handles answer to the new numbers. The reverb is
    /// the exception (its impulse is baked into the convolver), and must ASK for a rebuild.
    #[test]
    fn tune_keeps_nodes_and_the_reverb_asks_out() {
        let ctx = OfflineAudioContext::new(1, 128, 44100.0);
        let mut echo = Fx::build(
            &ctx,
            &serde_json::from_str::<FxSet>(r#"{"type":"echo","p":{"time":0.2}}"#).unwrap(),
        )
        .unwrap();
        assert!(echo.tune(&serde_json::from_str::<FxSet>(r#"{"type":"echo","p":{"time":0.4}}"#).unwrap()));

        let mut verb = Fx::build(
            &ctx,
            &serde_json::from_str::<FxSet>(r#"{"type":"verb","p":{"size":1.0}}"#).unwrap(),
        )
        .unwrap();
        // mix moves: tunable in place.
        assert!(verb.tune(&serde_json::from_str::<FxSet>(r#"{"type":"verb","p":{"size":1.0,"mix":0.8}}"#).unwrap()));
        // size moves: new impulse needed, must refuse.
        assert!(!verb.tune(&serde_json::from_str::<FxSet>(r#"{"type":"verb","p":{"size":2.0}}"#).unwrap()));
    }
}
