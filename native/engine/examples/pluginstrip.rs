//! **One reese note through a real channel strip, with and without a real VST in the insert.**
//!
//!   cargo run --release --example pluginstrip -- apqualizr
//!
//! The last question standing on 2026-08-14. Mickey's reese distorts on every note the moment a
//! plugin sits on the channel — every device, every mode, and even with the plugin's own bypass
//! engaged — and it is clean the instant the plugin is REMOVED from the chain. Meanwhile:
//!   * the reese itself nulls against Chrome at -54 dB at his notes (tests/parity.test.js),
//!   * the plugin passes audio at exact unity through `process_io` (vsthost's fxprobe),
//!   * no underrun, no slow plugin block, no CPU spike appears in volt.log.
//!
//! So the fault is in neither the instrument nor the plugin, but in what the graph does when the
//! plugin NODE is spliced into the strip. This renders both versions offline — same note, same
//! strip, same master, plugin in vs plugin out — and prints what actually comes out.
use std::collections::HashMap;
use std::path::Path;

use volt_engine::mixer::{MixState, Mixer, TrackSet};
use volt_engine::vst::VstNode;
use volt_engine::{noise_buffer, trig, Ctx, Job};
use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
use web_audio_api::node::AudioNode;

fn params(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

/// Mickey's reese, his settings, his lowest note.
fn reese_job(t: f64, sr: f32) -> Job {
    Job {
        instr: "reese".into(),
        note: 14.0, // D-1
        vel: 0.8,
        params: params(&[
            ("voices", 1.84), ("detune", 50.0), ("cutoff", 147.0), ("res", 0.0),
            ("lfoRate", 0.35), ("lfoAmt", 0.35), ("drive", 0.14), ("sub", 0.0), ("rel", 0.09),
        ]),
        dur: 0.5,
        sr,
        t,
        rev: false,
        strip: None,
        mix: None,
    }
}

fn measure(label: &str, d: &[f32]) {
    let mut peak = 0.0f32;
    let mut over = 0usize;
    let mut sq = 0.0f64;
    for &s in d {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
        if a > 1.0 {
            over += 1;
        }
        sq += (s as f64) * (s as f64);
    }
    let rms = (sq / d.len() as f64).sqrt();
    println!("{label:<28} peak {peak:>7.4}   rms {rms:>7.4}   samples over 1.0: {over}");
}

fn main() {
    const SR: f32 = 48000.0;
    let want = std::env::args().nth(1).unwrap_or_else(|| "apqualizr".into()).to_lowercase();
    let secs = 1.5;
    let frames = (secs * SR as f64) as usize;

    // Find the plugin once, so both runs use the same class.
    let mut found: Option<(std::path::PathBuf, i32)> = None;
    for path in volt_vsthost::scan(Path::new(volt_vsthost::VST3_DIR)) {
        let fname = path.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !fname.contains(&want) {
            continue;
        }
        let Ok(module) = volt_vsthost::Module::open(&path) else { continue };
        for info in module.classes() {
            if info.category == "Audio Module Class" && !info.is_instrument() {
                found = Some((path.clone(), info.index));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }

    for with_plugin in [false, true] {
        let mut ctx = OfflineAudioContext::new(2, frames, SR);
        let noise = noise_buffer(&ctx);
        let c = Ctx::new(&ctx, &noise);
        let mut mixer = Mixer::new(&ctx);
        mixer.ensure(&ctx, 1);
        let mut ts = TrackSet::default();
        ts.vol = 0.27; // his reese channel
        ts.flt_f = 1.0;
        let state = MixState {
            tracks: vec![ts],
            master: 0.9,
            delay_time: 60.0 / 174.0 * 0.75,
            ..Default::default()
        };
        mixer.apply(&ctx, &state);

        // The plugin has to outlive the render, exactly as the engine's `fx_vsts` map does.
        let (_retire_tx, retire_rx) = std::sync::mpsc::channel();
        let mut _held: Option<VstNode> = None;
        if with_plugin {
            let Some((path, index)) = &found else {
                println!("(no plugin matching '{want}' — skipping the with-plugin run)");
                continue;
            };
            let module = volt_vsthost::Module::open(path).expect("module opens");
            let info = module.classes().into_iter().find(|i| i.index == *index).expect("class");
            let plugin = module.instantiate(&info, SR as f64, 512).expect("instantiates");
            let (node, _ed) = VstNode::from_plugin(&ctx, plugin, _retire_tx.clone()).expect("node");
            mixer.set_plugin_chain(0, &[node.node()]);
            _held = Some(node);
        }

        mixer.master.connect(&ctx.destination());
        let v = trig(&c, &reese_job(0.05, SR)).expect("reese builds");
        v.connect(mixer.input(0).expect("strip 0"));

        let buf = ctx.start_rendering_sync();
        measure(
            if with_plugin { "WITH the plugin node" } else { "plugin removed (clean)" },
            buf.get_channel_data(0),
        );
        drop(retire_rx);
    }
    volt_vsthost::hard_exit(0);
}
