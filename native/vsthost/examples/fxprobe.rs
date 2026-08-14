//! Prove a plugin EFFECT passes audio: load it, push a sine through `process_io`, measure.
//!
//! `cargo run --example fxprobe -- <name substring>` — picks the first non-instrument class whose
//! name matches, instantiates it, feeds it a -6 dB sine for a quarter second of blocks, and prints
//! the input and output peaks. An effect that works reads back within sanity of its input (an EQ
//! at defaults is near-unity; a compressor is somewhat less); one that does not is silence.
//!
//! Written for the day channel-FX hosting landed (2026-08-12), because the claim that mattered —
//! "the Waves stuff works now" — is exactly the kind that must be proven by audio, not by a load
//! that didn't crash. WaveShell is the reason this takes a name: one shell file exports hundreds
//! of classes, and picking by name is how a human asks for one.
//!
//! Safe to run while VOLT is open: no device is opened, no window is shown, everything is offline.

use std::path::Path;

fn main() {
    let want = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    if want.is_empty() {
        eprintln!("usage: fxprobe <name substring>");
        volt_vsthost::hard_exit(2);
    }

    let mut checked = 0usize;
    for path in volt_vsthost::scan(Path::new(volt_vsthost::VST3_DIR)) {
        if volt_vsthost::is_blacklisted(&path) {
            continue;
        }
        // Cheap pre-filter: only open files whose name plausibly matches, plus every WaveShell —
        // the class name ("SSLComp Stereo") shares nothing with the file name ("WaveShell1-VST3").
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
            checked += 1;
            println!("{} — {} ({})", c.name, path.display(), c.vendor);
            match module.instantiate(&c, 48000.0, 512) {
                Ok(mut p) => {
                    println!("  loaded: {} in, {} out, latency {}", p.in_channels, p.out_channels, p.latency);
                    let frames = 256usize;
                    let events = volt_vsthost::HostEvents::new();
                    let mut out: Vec<Vec<f32>> = (0..p.out_channels.max(2)).map(|_| vec![0.0; frames]).collect();
                    let mut in_pk = 0.0f32;
                    let mut out_pk = 0.0f32;
                    // ~0.25 s of blocks: past any fade-in, enough for a compressor to settle.
                    for b in 0..48 {
                        let sine: Vec<f32> = (0..frames)
                            .map(|i| {
                                let n = b * frames + i;
                                0.5 * (2.0 * std::f32::consts::PI * 220.0 * n as f32 / 48000.0).sin()
                            })
                            .collect();
                        for &s in &sine {
                            in_pk = in_pk.max(s.abs());
                        }
                        let input: Vec<&[f32]> = vec![&sine, &sine];
                        if let Err(e) = p.process_io(&events, &input, &mut out, frames) {
                            println!("  process FAILED: {e}");
                            break;
                        }
                        // Only the tail half counts toward the verdict — attack ramps are honest.
                        if b >= 24 {
                            for ch in &out {
                                for &s in ch {
                                    out_pk = out_pk.max(s.abs());
                                }
                            }
                        }
                    }
                    println!("  in peak {in_pk:.3} -> out peak {out_pk:.3}  {}",
                             if out_pk > 0.01 { "PASSES AUDIO" } else { "SILENT" });
                }
                Err(e) => println!("  load failed: {e}"),
            }
            if checked >= 4 {
                // A name that matches half of WaveShell would otherwise probe for ten minutes.
                println!("(stopping after {checked} matches)");
                volt_vsthost::hard_exit(0);
            }
        }
    }
    println!("{checked} matching effect class(es) probed");
    // Plugins hang or crash in DLL teardown; end hard, as every host tool here does.
    volt_vsthost::hard_exit(0);
}
