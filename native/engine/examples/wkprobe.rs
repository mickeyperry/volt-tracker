//! Prove the WORKLET WRAPPER around a plugin passes audio clean — the layer fxprobe skips.
//!
//! `cargo run --example wkprobe -- <name substring>` loads the first matching EFFECT exactly as
//! the desktop app does, wraps it in `VstNode` (the AudioWorkletNode the mixer splices into a
//! strip), renders a -1 dB sine through it in an OfflineAudioContext, and prints input peak,
//! output peak, and the residue after subtracting the input. fxprobe already proves the PLUGIN
//! is clean through `process_io` directly; this proves the block-by-block worklet plumbing —
//! bus arrays, mono/stereo handling, buffer reuse — adds nothing of its own.
//!
//! Written for "putting apq again over the channel the extreme distortion is getting back"
//! (2026-08-13): a transparent EQ distorting means the pipe, not the plugin, and this is the
//! only piece of pipe fxprobe does not cover.

use std::path::Path;
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};

fn main() {
    let want = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    if want.is_empty() {
        eprintln!("usage: wkprobe <effect name substring>");
        volt_vsthost::hard_exit(2);
    }
    let sr = 48000.0f32;

    let mut found = false;
    'files: for path in volt_vsthost::scan(Path::new(volt_vsthost::VST3_DIR)) {
        if volt_vsthost::is_blacklisted(&path) {
            continue;
        }
        let fname = path.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !fname.contains(&want) && !fname.contains("waveshell") {
            continue;
        }
        let Ok(module) = volt_vsthost::Module::open(&path) else { continue };
        for c in module.classes() {
            if !c.category.contains("Audio Module") || c.is_instrument() {
                continue;
            }
            if !c.name.to_lowercase().contains(&want) {
                continue;
            }
            found = true;
            println!("{} — {}", c.name, path.display());
            let plugin = match module.instantiate(&c, sr as f64, 512) {
                Ok(p) => p,
                Err(e) => {
                    println!("  did not load: {e}");
                    continue;
                }
            };
            let latency = plugin.latency as usize;

            // The graph the mixer builds, in miniature: stereo source → VstNode → destination.
            let ctx = OfflineAudioContext::new(2, 48000, sr);
            let (retire_tx, _retire_rx) = std::sync::mpsc::channel();
            let (node, _editor) = match volt_engine::vst::VstNode::from_plugin(&ctx, plugin, retire_tx) {
                Ok(v) => v,
                Err(e) => {
                    println!("  no node: {e}");
                    continue;
                }
            };
            // A stereo test signal loud enough to expose clipping and asymmetric enough to expose
            // channel swaps: left = 220 Hz sine, right = the same sine at half gain.
            let mut buf = ctx.create_buffer(2, 48000, sr);
            for ch in 0..2 {
                let amp = if ch == 0 { 0.9f32 } else { 0.45 };
                let data: Vec<f32> = (0..48000)
                    .map(|i| amp * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / sr).sin())
                    .collect();
                buf.copy_to_channel(&data, ch);
            }
            let mut src = ctx.create_buffer_source();
            src.set_buffer(buf.clone());
            src.connect(node.node());
            node.node().connect(&ctx.destination());
            src.start_at(0.0);
            let mut ctx = ctx;
            let out = ctx.start_rendering_sync();

            // Compare the middle second (skip fades/warmup), latency-aligned.
            for ch in 0..2 {
                let inp = buf.get_channel_data(ch);
                let o = out.get_channel_data(ch);
                let (a, b) = (8000usize, 40000usize);
                let in_pk = inp[a..b].iter().fold(0.0f32, |m, s| m.max(s.abs()));
                let out_pk = o[a + latency..b + latency].iter().fold(0.0f32, |m, s| m.max(s.abs()));
                let mut resid = 0.0f64;
                let mut power = 0.0f64;
                for i in a..b {
                    let d = (o[i + latency] - inp[i]) as f64;
                    resid += d * d;
                    power += (inp[i] as f64) * (inp[i] as f64);
                }
                let db = 10.0 * (resid / power.max(1e-12)).log10();
                println!(
                    "  ch{ch}: in {in_pk:.3} -> out {out_pk:.3}, residue {db:.1} dB {}",
                    if db < -40.0 { "CLEAN" } else if out_pk < 0.001 { "SILENT" } else { "DIRTY" }
                );
            }
            break 'files;
        }
    }
    if !found {
        println!("no matching effect");
    }
    volt_vsthost::hard_exit(0);
}
