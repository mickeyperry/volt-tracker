//! **Is the engine's own output distorted by NOTE DENSITY?** Offline, measurable, no hardware.
//!
//!   cargo run --release --example denseprobe
//!
//! Mickey, 2026-08-14: *"when im putting notes in 16/1 its distorts and when its like every bar -
//! its not"* — with zero underruns and no slow plugin in the log, so it is not the audio thread
//! missing deadlines. That leaves the SIGNAL, and this measures it: the same reese line rendered
//! through the real channel strip and master at one note per bar, then per beat, then per 16th,
//! then per 32nd. It prints the peak, how many samples sit at full scale, and the DC offset.
//!
//! A peak that climbs with density means voices are summing where they should be cutting; samples
//! pinned at 1.0 mean the master is clipping rather than limiting. Either is audible as exactly
//! what he described, and neither can be seen in a parity null test against Chrome — both engines
//! would do it.
use std::collections::HashMap;

use volt_engine::mixer::{MixState, Mixer, TrackSet};
use volt_engine::{noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::AudioNode;

fn params(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

/// One take: `notes` evenly spaced across `secs`, through strip 0 and the master.
fn run(label: &str, spacing: f64, secs: f64) {
    const SR: f32 = 48000.0;
    let frames = (secs * SR as f64) as usize;
    let mut ctx = OfflineAudioContext::new(2, frames, SR);
    let noise = noise_buffer(&ctx);
    let c = Ctx::new(&ctx, &noise);

    let mut mixer = Mixer::new(&ctx);
    mixer.ensure(&ctx, 2);
    // Mickey's own reese channel, near enough: fader 0.39, master 0.9.
    let mut ts = TrackSet::default();
    ts.vol = 0.39;
    let state = MixState {
        tracks: vec![ts.clone(), ts.clone()],
        master: 0.9,
        delay_time: 60.0 / 174.0 * 0.75, // what setDelayTime() derives from the tempo
        ..Default::default()
    };
    mixer.apply(&ctx, &state);

    // His dnb5 reese preset.
    let p = params(&[
        ("voices", 3.0), ("detune", 50.0), ("cutoff", 147.0), ("res", 0.0),
        ("lfoRate", 0.35), ("lfoAmt", 0.35), ("drive", 0.576), ("sub", 0.027), ("rel", 0.09),
    ]);

    let mut live: Option<volt_engine::Voice> = None;
    let mut retired: Vec<(f64, volt_engine::Voice)> = Vec::new();
    let mut t = 0.05;
    let mut n = 0;
    while t < secs - 0.2 {
        // exactly what the audio thread does per cell: cut the lane, build, connect
        if let Some(mut old) = live.take() {
            old.cut(t);
            retired.push((old.quiet_by(t), old));
        }
        retired.retain(|(quiet, _)| *quiet > t);
        let job = Job {
            instr: "reese".into(), note: 24.0, vel: 0.8, params: p.clone(),
            dur: spacing, sr: SR, t, rev: false, strip: None, mix: None,
        };
        let v = trig(&c, &job).expect("reese builds");
        if std::env::args().any(|a| a == "nomix") {
            v.connect(&ctx.destination());
        } else {
            match mixer.input(0) {
                Some(input) => v.connect(input),
                None => v.connect(&ctx.destination()),
            }
        }
        live = Some(v);
        n += 1;
        t += spacing;
    }
    if !std::env::args().any(|a| a == "nomix") {
        mixer.master.connect(&ctx.destination());
    }

    let buf = ctx.start_rendering_sync();
    let d = buf.get_channel_data(0);
    let mut peak = 0.0f32;
    let mut at_full = 0usize;
    let mut sum = 0.0f64;
    let mut sq = 0.0f64;
    for &s in d.iter() {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
        if a >= 0.999 {
            at_full += 1;
        }
        sum += s as f64;
        sq += (s as f64) * (s as f64);
    }
    let rms = (sq / d.len() as f64).sqrt();
    println!(
        "{label:<22} {n:>4} notes   peak {peak:>6.3}   rms {rms:>6.4}   \
         at-full {at_full:>6}   dc {:>8.5}   live+retired {}",
        sum / d.len() as f64,
        retired.len() + live.iter().count()
    );
}

fn main() {
    println!("the same reese line, four densities, through the real strip and master\n");
    let secs = 4.0;
    run("one note per bar", 60.0 / 174.0 * 4.0, secs);
    run("per beat (1/4)", 60.0 / 174.0, secs);
    run("per 16th", 60.0 / 174.0 / 4.0, secs);
    run("per 32nd", 60.0 / 174.0 / 8.0, secs);
}
