//! Find out what is actually on this machine, and whether it makes a sound.
//!
//!   vstprobe                       — scan and list every plugin, both formats, instruments marked
//!   vstprobe <name fragment>       — load the first match, play a chord, report what came out
//!   vstprobe <name fragment> <wav> — the same, written to a WAV so it can be listened to
//!
//! This is the "does the host work at all" tool, and it is a separate binary from the engine on
//! purpose: a plugin that crashes on load takes this process down and nothing else. Running it over
//! every plugin is how we find out which ones are safe to host before one of them is holding
//! Mickey's song. Since 2026-08-10 that includes VST2: the 64-bit DLLs in the classic folders are
//! listed and playable exactly like the VST3s, and the 32-bit ones are *counted out loud* — a
//! 64-bit process cannot load them, and a scan that dropped them silently would look broken.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use volt_vsthost::{hard_exit, read_blacklist, scan, scan_vst2, HostEvents, Module, PluginInfo,
                   VST3_DIR};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(VST3_DIR);

    // The blacklist: plugins that crashed or hung a scan before. Skipped HERE TOO — this binary is
    // where hangs are diagnosed, but an unattended list run must not sit inside Bionic Delay's
    // LoadLibrary forever — and said out loud below, never silently.
    let banned = read_blacklist();
    let is_banned = |p: &Path| banned.iter().any(|b| b.as_str() == p.to_string_lossy());
    let mut paths = scan(&dir);
    paths.retain(|p| !is_banned(p));
    let mut v2 = scan_vst2();
    let banned_count = banned.len();
    v2.dlls.retain(|p| !is_banned(p));

    if args.len() < 2 {
        list(&paths, &v2, banned_count);
        return;
    }

    let want = args[1].to_lowercase();
    let out_wav = args.get(2).map(PathBuf::from);

    // Search instruments first: "vstprobe vital" should reach the synth, not a plugin that merely
    // has the word in its vendor string. VST3 is searched before VST2 — where a plugin ships both
    // (Jup-8000 V does), the newer format is the one VOLT would rather host.
    let mut found: Option<(Arc<Module>, PluginInfo)> = None;
    for p in &paths {
        let Ok(module) = Module::open(p) else { continue };
        let classes = module.classes();
        if let Some(info) = classes
            .iter()
            .find(|c| c.is_instrument() && c.name.to_lowercase().contains(&want))
            .or_else(|| classes.iter().find(|c| c.name.to_lowercase().contains(&want)))
        {
            found = Some((module, info.clone()));
            break;
        }
    }
    if found.is_none() {
        // VST2: match on the file name first, so only candidates are loaded — a full open of all
        // 200+ DLLs to answer one search would turn every typo into a minute of plugin roulette.
        for p in v2.dlls.iter().filter(|p| stem_matches(p, &want)) {
            let Ok(module) = Module::open(p) else { continue };
            if let Some(info) = module.classes().first().cloned() {
                found = Some((module, info));
                break;
            }
        }
    }

    let Some((module, info)) = found else {
        eprintln!("no plugin matching `{want}` — run with no arguments to see the list");
        std::process::exit(1);
    };

    play(&module, &info, out_wav.as_deref());
}

fn stem_matches(p: &Path, want: &str) -> bool {
    p.file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains(want))
        .unwrap_or(false)
}

fn list(paths: &[PathBuf], v2: &volt_vsthost::Vst2Scan, banned: usize) {
    let mut ok = 0;
    let mut instruments = 0;
    let mut broken = Vec::new();

    for p in paths {
        match Module::open(p) {
            Ok(m) => {
                for c in m.classes() {
                    // Only audio classes are interesting: a plugin also exports its own controller
                    // and helper classes, and listing those is noise.
                    if !c.category.contains("Audio Module") {
                        continue;
                    }
                    ok += 1;
                    let kind = if c.is_instrument() {
                        instruments += 1;
                        "INSTR"
                    } else {
                        "  fx "
                    };
                    println!("{kind}  VST3  {:<34} {:<22} {}", trunc(&c.name, 33),
                             trunc(&c.vendor, 21), c.subcategories);
                }
            }
            Err(e) => broken.push(e),
        }
    }

    // The VST2s. Opening each one is the only way to learn its name and whether it is a synth —
    // and finding the ones that crash on open is this binary's actual job.
    let mut ok2 = 0;
    for p in &v2.dlls {
        match Module::open(p) {
            Ok(m) => {
                for c in m.classes() {
                    ok2 += 1;
                    let kind = if c.is_instrument() {
                        instruments += 1;
                        "INSTR"
                    } else {
                        "  fx "
                    };
                    println!("{kind}  VST2  {:<34} {:<22} {}", trunc(&c.name, 33),
                             trunc(&c.vendor, 21), c.subcategories);
                }
            }
            Err(e) => broken.push(e),
        }
    }

    println!();
    println!("{ok} VST3 classes and {ok2} VST2 plugins, {instruments} instruments in all \
              ({} VST3 files, {} VST2 DLLs)", paths.len(), v2.dlls.len());
    if v2.skipped_32bit > 0 {
        println!("{} VST2 DLLs skipped: 32-bit, and a 64-bit VOLT cannot load them (they need a \
                  bridge process — a separate project)", v2.skipped_32bit);
    }
    if banned > 0 {
        println!("{banned} blacklisted (crashed or hung a scan before — see \
                  %LOCALAPPDATA%\\VOLT\\vst-blacklist.txt)");
    }
    if !broken.is_empty() {
        println!("\n{} would not load:", broken.len());
        for b in &broken {
            println!("  {b}");
        }
    }

    // A scan loads every plugin on the machine; falling off the end of main would hand all of
    // them to Windows' DLL teardown at once. See hard_exit.
    hard_exit(0);
}

fn play(module: &Arc<Module>, info: &PluginInfo, wav: Option<&Path>) {
    const SR: f64 = 44100.0;
    const BLOCK: usize = 256;
    const SECONDS: f64 = 2.0;

    println!("{} — {} [{}]", info.name, info.vendor, info.subcategories);

    let mut plugin = match module.instantiate(info, SR, BLOCK as i32) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not start it: {e}");
            std::process::exit(2);
        }
    };
    println!("  in {} ch / out {} ch, {} event-in bus(es), {} parameters, {} samples latency",
             plugin.in_channels, plugin.out_channels, plugin.event_in_buses, plugin.parameters,
             plugin.latency);
    if plugin.event_in_buses == 0 {
        println!("  NO EVENT INPUT — this plugin cannot be sent notes at all.");
    }

    if plugin.out_channels == 0 {
        eprintln!("  no audio output bus — nothing to record");
        std::process::exit(3);
    }

    let events = HostEvents::new();
    let mut out: Vec<Vec<f32>> = (0..plugin.out_channels).map(|_| vec![0.0; BLOCK]).collect();
    let total = (SR * SECONDS) as usize;
    let mut recorded: Vec<Vec<f32>> = (0..plugin.out_channels).map(|_| Vec::with_capacity(total)).collect();

    // A minor triad, held for a second and released — enough to tell a synth that is working from
    // one that is merely not crashing, and short enough that the tail fits in the render.
    let chord = [48.0f32, 51.0, 55.0];
    let on_at = (SR * 0.1) as usize;
    let off_at = (SR * 1.1) as usize;

    let mut pos = 0usize;
    while pos < total {
        let frames = BLOCK.min(total - pos);
        events.clear();
        for (i, note) in chord.iter().enumerate() {
            // Sample-accurate placement inside the block: the whole point of an event list is that
            // a note does not have to land on a block boundary, and if this is wrong it is wrong by
            // up to 5.8 ms, which is exactly the latency all this work is for.
            if on_at >= pos && on_at < pos + frames {
                events.note_on(*note, 0.8 - i as f32 * 0.05, (on_at - pos) as i32);
            }
            if off_at >= pos && off_at < pos + frames {
                events.note_off(*note, (off_at - pos) as i32);
            }
        }

        if let Err(e) = plugin.process(&events, &mut out, frames) {
            eprintln!("  {e}");
            std::process::exit(4);
        }
        for (ch, buf) in recorded.iter_mut().zip(out.iter()) {
            ch.extend_from_slice(&buf[..frames]);
        }
        pos += frames;
    }

    let peak = recorded.iter().flat_map(|c| c.iter()).fold(0.0f32, |a, &b| a.max(b.abs()));
    let rms = {
        let n: usize = recorded.iter().map(|c| c.len()).sum();
        let s: f64 = recorded.iter().flat_map(|c| c.iter()).map(|&x| (x as f64) * (x as f64)).sum();
        (s / n as f64).sqrt()
    };
    println!("  rendered {:.1}s: peak {peak:.4}, rms {rms:.5}", SECONDS);
    if peak < 1e-6 {
        println!("  SILENT — it loaded and processed, but produced nothing. Some synths need a\n\
                  \x20 preset loaded through IEditController before they make a sound.");
    } else {
        println!("  it plays.");
    }

    if let Some(path) = wav {
        write_wav(path, &recorded, SR as u32);
        println!("  wrote {}", path.display());
    }

    // Deliberate: teardown is where several plugins fall over, and a crash after the last line of
    // output is indistinguishable from a clean exit unless something prints on the other side of it.
    drop(plugin);
    println!("  released cleanly.");

    // Do not fall off the end of main: that hands control to Windows' DLL teardown, which is where
    // plugins hang or crash long after their work is done. See hard_exit.
    hard_exit(0);
}

/// 16-bit WAV, same format as the engine's `wav::encode`, kept here so this crate does not depend
/// on the engine just to save a file.
fn write_wav(path: &Path, chans: &[Vec<f32>], sr: u32) {
    let n = chans[0].len();
    let ch = chans.len() as u16;
    let bytes = (n * ch as usize * 2) as u32;
    let mut v = Vec::with_capacity(44 + bytes as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + bytes).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&ch.to_le_bytes());
    v.extend_from_slice(&sr.to_le_bytes());
    v.extend_from_slice(&(sr * ch as u32 * 2).to_le_bytes());
    v.extend_from_slice(&(ch * 2).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&bytes.to_le_bytes());
    for i in 0..n {
        for c in chans {
            let s = (c[i].clamp(-1.0, 1.0) * 32767.0) as i16;
            v.extend_from_slice(&s.to_le_bytes());
        }
    }
    let _ = std::fs::write(path, v);
}

fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("{}…", &s[..n - 1]) }
}
