//! Play through the full mixer on a REAL device, the way the desktop app does.
//!
//!   cargo run --release --example livemix              default device
//!   cargo run --release --example livemix -- nomix     same notes, straight to the destination
//!   cargo run --release --example livemix -- nodelay   mixer, but the ping-pong bus unbuilt
//!   cargo run --release --example livemix -- noverb    mixer, but the convolver unbuilt
//!
//! **Why this exists.** Mickey pressed Play with the new mixer and the app hung. Every test the
//! mixer had was on an `OfflineAudioContext` rendered with `start_rendering_sync()` — a completely
//! different driver from a live callback, with no deadline and no render thread. So "the mixer is
//! cheap" was measured in the one place that could not show this class of bug.
//!
//! This runs the app's actual arrangement: a live `AudioContext`, strips feeding a master, the send
//! buses live, notes scheduled through it, and a mixer snapshot streaming in at the rate the UI
//! sends one. The switches bisect it — if `nomix` survives and the default hangs, it is the mixer;
//! if `nodelay` survives, it is the feedback cycle; if `noverb` survives, it is the convolver.
//!
//! It prints a heartbeat every 250 ms. **A run that stops printing has reproduced the hang**, and
//! the last line says which stage it reached.

use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

use volt_engine::mixer::{MixState, Mixer, TrackSet};
use volt_engine::{noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{
    AudioContext, AudioContextLatencyCategory, AudioContextOptions, BaseAudioContext,
};
use web_audio_api::node::AudioNode;

fn params(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let (nomix, nodelay, noverb) = (has("nomix"), has("nodelay"), has("noverb"));

    let t0 = Instant::now();
    let step = |what: &str| {
        println!("  [{:>7.0} ms] {what}", t0.elapsed().as_secs_f64() * 1000.0);
        let _ = std::io::stdout().flush();
    };

    // Pick a device explicitly rather than taking the default: run from a service-like context
    // there may not BE a default, and the app always names one anyway.
    let want = args.iter().skip(1).find(|a| {
        !matches!(a.as_str(), "nomix" | "nodelay" | "noverb")
    });
    let outs: Vec<_> = web_audio_api::media_devices::enumerate_devices_sync()
        .into_iter()
        .filter(|d| d.kind() == web_audio_api::media_devices::MediaDeviceInfoKind::AudioOutput)
        .collect();
    let dev = want
        .and_then(|w| outs.iter().find(|d| d.label().to_lowercase().contains(&w.to_lowercase())))
        .or_else(|| outs.first());
    let Some(dev) = dev else {
        eprintln!("no output devices at all");
        std::process::exit(2);
    };
    step(&format!("opening {}", dev.label()));
    let ctx = AudioContext::new(AudioContextOptions {
        latency_hint: AudioContextLatencyCategory::Interactive,
        sink_id: dev.device_id().to_string(),
        ..AudioContextOptions::default()
    });
    step(&format!(
        "open at {} Hz, {:.1} ms",
        ctx.sample_rate(),
        (ctx.base_latency() + ctx.output_latency()) * 1000.0
    ));

    let noise = noise_buffer(&ctx);
    let c = Ctx::new(&ctx, &noise);

    let mut mixer = None;
    if !nomix {
        step("building the mixer  >>> if this is the last line, Master::new hangs <<<");
        let mut m = Mixer::new_with(&ctx, !nodelay, !noverb);
        m.master.connect(&ctx.destination());
        step("mixer built and connected to the device");

        step("applying a 32-track mix state  >>> if last, the first apply hangs <<<");
        m.apply(
            &ctx,
            &MixState {
                tracks: vec![TrackSet::default(); 32],
                master: 0.75,
                delay_time: 0.35,
                ..Default::default()
            },
        );
        step("mix applied, 32 strips live");
        mixer = Some(m);
    }

    // Four bars of kick and hat, scheduled ahead exactly as the engine's clock does.
    step("scheduling notes  >>> if last, connecting a voice to a strip hangs <<<");
    let mut held = Vec::new();
    for i in 0..16 {
        let t = ctx.current_time() + 0.2 + i as f64 * 0.25;
        let (instr, track, p) = if i % 4 == 0 {
            ("kick", 0usize, params(&[("pitch", 50.0), ("punch", 0.6), ("decay", 0.4), ("drive", 0.3)]))
        } else {
            ("hat", 1usize, params(&[("decay", 0.05), ("tone", 0.7)]))
        };
        let job = Job {
            instr: instr.into(), note: 48.0, vel: 0.8, params: p,
            dur: 0.4, sr: ctx.sample_rate(), t, rev: false, strip: None, mix: None,
        };
        match trig(&c, &job) {
            Ok(v) => {
                match mixer.as_mut() {
                    Some(m) => {
                        m.ensure(&ctx, track + 1);
                        match m.input(track) {
                            Some(input) => v.connect(input),
                            None => v.connect(&ctx.destination()),
                        }
                    }
                    None => v.connect(&ctx.destination()),
                }
                held.push(v);
            }
            Err(e) => println!("  trig: {e}"),
        }
    }
    step("notes scheduled");

    // And the mixer state streaming in at the rate the UI sends it — 25 a second, wholesale.
    step("streaming mix updates for 4 s  >>> if last, apply-under-playback hangs <<<");
    let start = Instant::now();
    let mut sent = 0;
    let mut beat = 0;
    while start.elapsed().as_secs_f64() < 4.0 {
        if let Some(m) = mixer.as_mut() {
            let mut tracks = vec![TrackSet::default(); 32];
            // Move something, so this is not a no-op the crate could optimise away.
            tracks[0].vol = 0.5 + 0.3 * (start.elapsed().as_secs_f64() * 3.0).sin() as f32;
            tracks[0].dly = 0.4;
            tracks[0].rvb = 0.3;
            m.apply(&ctx, &MixState { tracks, master: 0.75, delay_time: 0.35, ..Default::default() });
            sent += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
        if start.elapsed().as_secs_f64() * 4.0 > beat as f64 {
            beat += 1;
            step(&format!("alive — {sent} mix updates sent"));
        }
    }

    step(&format!("SURVIVED — {sent} mix updates, {} voices", held.len()));
    volt_vsthost::hard_exit(0);
}
