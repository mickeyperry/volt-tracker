//! Live output — the number the whole native plan rests on, measured through the real engine.
//!
//!   live                 list output devices and exit
//!   live <n|substring>   open that device and play a bar of drums
//!
//! `native/probe` measures with cpal directly, and what it prints is a callback *period*, not a
//! latency — see native/README.md, that distinction cost us a wrong headline. This asks the
//! question that actually matters: what does the *engine we are building on* report as output
//! latency, the same quantity the browser reports as `baseLatency + outputLatency`? Measured
//! here: 12 ms through the nio on WASAPI, against 58 ms in the browser.
//!
//! Build with ASIO (needs the Steinberg SDK and clang — see native/README.md):
//!   $env:CPAL_ASIO_DIR="C:\Users\Mickey\sdk\ASIOSDK"; $env:LIBCLANG_PATH="C:\Program Files\LLVM\bin"
//!   cargo run --release --features asio --bin live

use std::collections::HashMap;
use volt_engine::{clock::Clock, noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{
    AudioContext, AudioContextLatencyCategory, AudioContextOptions, BaseAudioContext,
};
use web_audio_api::media_devices::{enumerate_devices_sync, MediaDeviceInfoKind};

/// One instrument's parameters, as `beta2.html`'s presets define them.
fn params(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn main() {
    let outs: Vec<_> = enumerate_devices_sync()
        .into_iter()
        .filter(|d| d.kind() == MediaDeviceInfoKind::AudioOutput)
        .collect();

    let arg = std::env::args().nth(1);
    let Some(arg) = arg else {
        println!("output devices:\n");
        for (i, d) in outs.iter().enumerate() {
            println!("  [{i}]  {}", d.label());
        }
        println!("\nplay through one with:  live <n|substring>");
        return;
    };

    // Accept an index or any distinctive part of the name — on Windows the ASIO driver worth
    // having is rarely the one the OS considers default, so this is always going to be explicit.
    let picked = arg
        .parse::<usize>()
        .ok()
        .and_then(|i| outs.get(i))
        .or_else(|| {
            outs.iter()
                .find(|d| d.label().to_lowercase().contains(&arg.to_lowercase()))
        });
    let Some(dev) = picked else {
        eprintln!("no output device matching `{arg}` — run `live` with no arguments to list them");
        std::process::exit(2);
    };
    println!("opening: {}", dev.label());

    // ---- thread mode ------------------------------------------------------------------------
    // `live <device> thread` opens the device from a SPAWNED thread instead of the main one, which
    // is exactly what the desktop app does (`eng_open` hands the context to its own audio thread).
    // It exists because the desktop app could not open ASIO4ALL while this binary could, and the
    // threading model was the only structural difference between them. If this mode fails where
    // plain `live <device>` succeeds, the bug is the thread, not the driver.
    if std::env::args().nth(2).as_deref() == Some("thread") {
        println!("
  re-opening the same device from a spawned thread…");
        let id = dev.device_id().to_string();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let ctx2 = AudioContext::new(AudioContextOptions {
                latency_hint: AudioContextLatencyCategory::Interactive,
                sink_id: id,
                ..AudioContextOptions::default()
            });
            let msg = format!(
                "  opened on the spawned thread in {:.2} s — {:.2} ms latency at {} Hz",
                t0.elapsed().as_secs_f64(),
                (ctx2.base_latency() + ctx2.output_latency()) * 1000.0,
                ctx2.sample_rate()
            );
            let _ = tx.send(msg);
            std::thread::sleep(std::time::Duration::from_millis(500));
        });
        match rx.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(m) => println!("{m}"),
            Err(_) => println!("  FAILED — the spawned thread did not open the device in 20 s.
                                  That is the desktop app's bug, reproduced."),
        }
        volt_vsthost::hard_exit(0);
    }


    let ctx = AudioContext::new(AudioContextOptions {
        latency_hint: AudioContextLatencyCategory::Interactive,
        sink_id: dev.device_id().to_string(),
        ..AudioContextOptions::default()
    });

    let sr = ctx.sample_rate();
    let base = ctx.base_latency();
    let out = ctx.output_latency();
    println!("\n  sample rate     {sr} Hz");
    println!("  base latency    {:.2} ms", base * 1000.0);
    println!("  output latency  {:.2} ms", out * 1000.0);
    println!("  TOTAL           {:.2} ms", (base + out) * 1000.0);
    println!("\n  for comparison — Web Audio in the browser on this machine: 58 ms");

    // The same shared noise buffer the offline engine builds, from the same seed — so a hat here
    // is sample-for-sample the hat the parity harness measured.
    let noise = noise_buffer(&ctx);
    let c = Ctx::new(&ctx, &noise);

    // ---- sample mode -------------------------------------------------------------------------
    // `live <device> sample` plays a sampler voice instead of the drum pattern.
    //
    // This exists because of what Mickey actually asked for: samples out of ASIO, nothing else.
    // The parity harness proves the ported sampler matches the browser, but it proves it OFFLINE —
    // and offline says nothing about whether a buffer source survives a real-time ASIO callback at
    // a 256-frame buffer. This is the smallest thing that answers that, without needing a WAV
    // decoder in here: the same seeded-noise fixture the null test uses, played live.
    if std::env::args().nth(2).as_deref() == Some("sample") {
        let len = (sr as usize) / 2; // half a second
        let mut smp = ctx.create_buffer(1, len, sr);
        let mut rnd = volt_engine::seed_rnd(0xB0B);
        let data: Vec<f32> = (0..len).map(|_| (rnd() * 2.0 - 1.0) as f32).collect();
        smp.copy_to_channel(&data, 0);

        let cs = Ctx::new(&ctx, &noise).with_sample(&smp);
        let mut held = Vec::new();
        // Four notes: the root, then transposed, so the playback rate is exercised live rather
        // than only in the offline null test.
        for (i, note) in [48.0f32, 55.0, 60.0, 43.0].iter().enumerate() {
            let job = Job {
                instr: "sampler".into(),
                note: *note,
                vel: 0.8,
                params: params(&[
                    ("root", 48.0), ("rel", 0.04), ("amp", 1.0),
                    ("fin", 0.002), ("fout", 0.02),
                ]),
                dur: 0.5,
                sr,
                t: ctx.current_time() + 0.15 + i as f64 * 0.4,
                rev: false,
                strip: None,
                mix: None,
            };
            match trig(&cs, &job) {
                Ok(v) => { v.connect(&ctx.destination()); held.push(v); }
                Err(e) => { eprintln!("  sampler: {e}"); std::process::exit(4); }
            }
        }
        println!("\n  playing 4 sampler notes (root, +7, +12, -5)…");
        std::thread::sleep(std::time::Duration::from_secs_f64(2.2));
        println!("  done — {} sampler voices through {:.2} ms of latency.", held.len(), out * 1000.0);
        return;
    }

    // ---- VST mode ----------------------------------------------------------------------------
    // `live <device> vst <name>` loads a VST3 instrument and plays a pattern through it, out of the
    // chosen device. This is the first time a plugin makes a sound inside VOLT.
    //
    // What is worth noticing: the plugin is not special-cased anywhere below. It is an
    // AudioWorkletNode, it connects to the destination like an oscillator would, and the SAME clock
    // that schedules `trig()` voices schedules its notes. That is the payoff of hosting it as a
    // node — when the channel strip is in the path, the plugin goes in the insert slot with no
    // further plumbing.
    if std::env::args().nth(2).as_deref() == Some("vst") {
        let want = std::env::args().nth(3).unwrap_or_else(|| "Vital".to_string()).to_lowercase();

        let mut found = None;
        for p in volt_vsthost::scan(std::path::Path::new(volt_vsthost::VST3_DIR)) {
            let Ok(module) = volt_vsthost::Module::open(&p) else { continue };
            if let Some(info) = module
                .classes()
                .into_iter()
                .find(|c| c.is_instrument() && c.name.to_lowercase().contains(&want))
            {
                found = Some((module, info));
                break;
            }
        }
        let Some((module, info)) = found else {
            eprintln!("no VST3 instrument matching `{want}` — try: vstprobe");
            std::process::exit(5);
        };
        println!("\n  plugin: {} — {}", info.name, info.vendor);

        // 512 is a ceiling, not the block size: the render quantum is 128, and a plugin told to
        // expect at most 512 is happy to be handed less. Being handed MORE than it was set up for
        // is the error case, which is why this is generous.
        // The editor half is dropped here on purpose: `live` is a headless measuring tool with no
        // message loop, and a plugin window needs one. Opening editors is the desktop app's job.
        //
        // The retirement channel's receiver is dropped immediately for the same reason — nothing
        // here outlives the measurement, and a failed send just means the plugin is dropped on the
        // audio thread, which is where this tool created it anyway.
        let (retire, _) = std::sync::mpsc::channel();
        let (vst, _editor) = match volt_engine::vst::VstNode::new(&ctx, &module, &info, 512, retire) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  could not start it: {e}");
                std::process::exit(6);
            }
        };
        println!("  reports {} samples of latency", vst.latency);
        use web_audio_api::node::AudioNode;
        vst.node().connect(&ctx.destination());

        // A meter on the plugin's output, because "it ran without crashing" and "it made a sound"
        // are different claims and only one of them is worth reporting. A plugin that loads, is
        // sent notes and outputs digital silence is a real and common failure — several synths
        // need a preset loaded through IEditController before they make any sound at all — and it
        // is indistinguishable from success unless something is watching the samples.
        let mut analyser = ctx.create_analyser();
        vst.node().connect(&analyser);

        // The same clock, the same row duration, the same lookahead as every other mode.
        const TICK: f64 = 0.030;
        const LOOKAHEAD: f64 = 0.160;
        let mut clock = Clock::new(120.0, 4.0, 16);
        clock.start(ctx.current_time() + 0.2, 0);
        let riff = [48.0f32, 55.0, 60.0, 63.0, 60.0, 55.0, 51.0, 55.0];
        let row_dur = clock.row_dur();

        println!("  playing 2 bars through it…");
        let end = ctx.current_time() + 2.0 * 16.0 * row_dur + 0.6;
        let mut peak = 0.0f32;
        let mut buf = vec![0.0f32; analyser.fft_size()];
        while ctx.current_time() < end {
            analyser.get_float_time_domain_data(&mut buf);
            peak = buf.iter().fold(peak, |a, &b| a.max(b.abs()));
            for due in clock.due(ctx.current_time(), LOOKAHEAD) {
                if due.row % 2 == 0 {
                    let note = riff[(due.row / 2) % riff.len()];
                    vst.note_on(note, 0.8, due.t);
                    // Held for just under two rows, so the synth's own release is audible instead
                    // of every note being cut by the next.
                    vst.note_off(note, due.t + row_dur * 1.8);
                }
            }
            std::thread::sleep(std::time::Duration::from_secs_f64(TICK));
        }
        if peak < 1e-4 {
            println!("  SILENT — it loaded and processed but output nothing. Some synths need a\n\
                      \x20 preset loaded through IEditController first.");
        } else {
            println!("  peak {peak:.4} at the plugin's output — it sounded.");
        }
        println!("  done — a VST3 instrument played from VOLT's clock at {:.2} ms.", out * 1000.0);
        // Released above; what is left is Windows unloading the plugin's DLL, which several
        // plugins do not survive and Jup-8000 V never returns from. See volt_vsthost::hard_exit.
        drop(vst);
        volt_vsthost::hard_exit(0);
    }

    // ---- the pattern -------------------------------------------------------------------------
    // A 16-row pattern in the shape VOLT stores one: a cell per row, or nothing. Four-on-the-floor
    // with offbeat hats. Every voice goes through the ported `trig()` — the same function the null
    // tests grade — rather than a copy kept in this file, which is why `Ctx` became generic.
    let kick_p = params(&[("pitch", 50.0), ("punch", 0.6), ("decay", 0.4), ("drive", 0.3)]);
    let hat_p = params(&[("decay", 0.06), ("tone", 0.7)]);
    let pattern: Vec<Option<(&str, f32, f32)>> = (0..16)
        .map(|i| {
            if i % 4 == 0 {
                Some(("kick", 48.0, 0.9))
            } else if i % 2 == 1 {
                Some(("hat", 60.0, 0.5))
            } else {
                None
            }
        })
        .collect();

    // ---- the transport -----------------------------------------------------------------------
    // `Clock` is `pump()`: a 30 ms tick that books every row falling inside the next 160 ms, each
    // at an absolute time. Nothing is ever played "now" — the tick decides what to schedule and
    // the audio clock decides when it sounds, which is what keeps timing exact when the thread
    // running this loop is late. Unit-tested in src/clock.rs, including ten minutes without drift.
    const TICK: f64 = 0.030;
    const LOOKAHEAD: f64 = 0.160;
    let bars = 2;

    let mut clock = Clock::new(120.0, 4.0, pattern.len());
    clock.start(ctx.current_time() + 0.15, 0);
    println!(
        "\n  playing {bars} bars at {} bpm, {} rows/beat ({:.1} ms per row)…",
        clock.bpm,
        clock.lpb,
        clock.row_dur() * 1000.0
    );

    // Voices are held until the end. A `GainNode` dropped while its note is still sounding takes
    // the note with it, so the tail of the last hat depends on this vector outliving the loop.
    let mut voices = Vec::new();
    let total = pattern.len() * bars;
    let mut fired = 0usize;

    while fired < total {
        for d in clock.due(ctx.current_time(), LOOKAHEAD) {
            if fired >= total {
                clock.stop();
                break;
            }
            fired += 1;
            let Some((instr, note, vel)) = pattern[d.row] else {
                continue;
            };
            let job = Job {
                instr: instr.to_string(),
                note,
                vel,
                params: if instr == "kick" { kick_p.clone() } else { hat_p.clone() },
                dur: clock.row_dur(),
                sr,
                rev: false,
                t: d.t,
                strip: None,
                mix: None,
            };
            match trig(&c, &job) {
                Ok(v) => {
                    v.connect(&ctx.destination());
                    voices.push(v);
                }
                Err(e) => eprintln!("  {instr}: {e}"),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(TICK));
    }

    // Everything is booked, but the last rows have not sounded yet — wait out the lookahead plus
    // the tail of the final note.
    std::thread::sleep(std::time::Duration::from_secs_f64(LOOKAHEAD + 0.6));
    println!("  done — {} voices, {fired} rows.", voices.len());
}

