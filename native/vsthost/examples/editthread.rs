//! Reproduce the DESKTOP APP's threading around a plugin editor, and time every step.
//!
//!   cargo run --release --example editthread -- sytrus
//!
//! **Why this exists and `editwin.rs` was not enough.** `editwin` creates the plugin and opens its
//! editor on one thread. The app does not: it loads the plugin on the **audio thread** (that is
//! where the graph lives) and opens the editor on the **main thread** (that is where a window's
//! messages are delivered) — and it keeps calling `process()` on the audio thread the whole time.
//! `editwin` passed 50/50 while the app hung on the first VST2 tried, and that difference is the
//! entire reason. A test that does not reproduce the threading proves nothing about the threading.
//!
//! Every step prints its elapsed time, so a hang is localised rather than merely observed. If the
//! run stops printing, the last line names the call that hung — which is the whole point.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Instant;

use volt_vsthost::{editor::Editor, hard_exit, read_blacklist, scan, scan_vst2, HostEvents, Module,
                   VST3_DIR};

use winapi::um::winuser::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};

/// The proposed fix, end to end: **create the plugin here, on the main thread**, hand it to an
/// audio thread to be processed, and open its editor back here.
///
/// This is what the desktop app should do. If a plugin passes this and hangs in the default mode,
/// the difference is the creating thread and nothing else.
fn main_thread_load(
    paths: &[std::path::PathBuf],
    step: &impl Fn(&str),
    no_proc: bool,
    stop: Arc<AtomicBool>,
    processing: Arc<AtomicBool>,
) {
    let mut found = None;
    for path in paths {
        step(&format!("opening {}", path.display()));
        let Ok(module) = Module::open(path) else {
            step("  module would not open");
            continue;
        };
        for info in module.classes() {
            if !info.category.contains("Audio Module") || !info.is_instrument() {
                continue;
            }
            step(&format!("  instantiating {}", info.name));
            match module.instantiate(&info, 44100.0, 512) {
                Ok(p) => {
                    found = Some((info.name.clone(), format!("{:?}", info.format), p));
                    break;
                }
                Err(e) => println!("  could not start {}: {e}", info.name),
            }
        }
        if found.is_some() {
            break;
        }
    }
    let Some((name, format, mut plugin)) = found else {
        eprintln!("no instrument matching that");
        return;
    };
    step(&format!("loaded {name} [{format}] on the MAIN thread"));

    let editor = plugin.take_editor();

    // Hand the audio half over, exactly as the app would: one move, one owner.
    //
    // **And hand it BACK when done.** A VST2 plugin must be destroyed on the thread that created
    // it, for the same reason it must be created there — measured: dropping Sytrus on the audio
    // thread exits 222, Sylenth1 segfaults. So the audio thread returns it rather than dropping
    // it, and the main thread does the destroying. This is the "non-RT reclamation" the
    // engineering bible prescribes, arrived at the hard way.
    let (retire_tx, retire_rx) = channel();
    let audio = {
        let (stop, processing) = (Arc::clone(&stop), Arc::clone(&processing));
        std::thread::spawn(move || {
            let events = HostEvents::new();
            let mut out: Vec<Vec<f32>> =
                (0..plugin.out_channels.max(1)).map(|_| vec![0.0; 256]).collect();
            processing.store(true, Ordering::Release);
            while !stop.load(Ordering::Acquire) {
                if !no_proc {
                    let _ = plugin.process(&events, &mut out, 256);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _ = retire_tx.send(plugin);
        })
    };
    while !processing.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    step("plugin moved to the audio thread and processing; opening its editor here");
    step("  >>> if this is the last line, the fix does NOT work <<<");

    let Some(handle) = editor else {
        step("no editor half");
        stop.store(true, Ordering::Release);
        let _ = audio.join();
        return;
    };
    match Editor::open(handle) {
        Ok(ed) => {
            let (w, h) = ed.size();
            step(&format!("EDITOR ATTACHED, window {w}×{h} — the fix works"));
            pump(3.0, step);
            step("tearing down");
            drop(ed);
            step("torn down cleanly");
        }
        Err(e) => step(&format!("editor refused: {e}")),
    }
    stop.store(true, Ordering::Release);
    let _ = audio.join();
    step("audio thread stopped, plugin handed back");
    match retire_rx.recv() {
        Ok(p) => {
            drop(p);
            step("PLUGIN DESTROYED ON THE MAIN THREAD — clean");
        }
        Err(_) => step("nothing came back to destroy"),
    }
}

/// Run a message loop for `secs`, reporting that it stayed responsive.
fn pump(secs: f64, step: &impl Fn(&str)) {
    let start = Instant::now();
    let mut ticks = 0u32;
    while start.elapsed().as_secs_f64() < secs {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
                ticks += 1;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    step(&format!("pumped {ticks} messages over {secs} s — the loop stayed responsive"));
}

fn main() {
    let want = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    if want.is_empty() {
        eprintln!("usage: editthread <name fragment> [noproc]");
        eprintln!("  noproc: load on the audio thread but do NOT call process() while the editor");
        eprintln!("          opens — isolates 'concurrent process' from 'opened on another thread'");
        std::process::exit(1);
    }
    // The decisive variable. With processing off, the ONLY difference from `editwin` is which
    // thread created the plugin; with it on, the plugin is also being called concurrently.
    let no_proc = std::env::args().any(|a| a == "noproc");
    // The PROPOSED FIX, modelled: create the plugin on the main thread, move it to the audio
    // thread to be processed, and open its editor back here. If this passes where the default
    // mode hangs, the fix is right and the app should be restructured to match.
    let main_load = std::env::args().any(|a| a == "mainload");

    let banned = read_blacklist();
    let mut paths: Vec<_> = scan(std::path::Path::new(VST3_DIR));
    paths.extend(scan_vst2().dlls);
    paths.retain(|p| {
        !banned.iter().any(|b| b.as_str() == p.to_string_lossy())
            && p.file_stem()
                .map(|s| s.to_string_lossy().to_lowercase().contains(&want))
                .unwrap_or(false)
    });
    if paths.is_empty() {
        eprintln!("no plugin file matching `{want}`");
        std::process::exit(1);
    }

    let t0 = Instant::now();
    // **Flushed after every line, and that is not fussiness.** Rust block-buffers stdout when it is
    // redirected to a file, so a process that crashes loses everything it "printed" — which turns a
    // harness whose entire job is to say *where* a plugin died into one that says nothing at all.
    // Cost this diagnosis half an hour of believing a crash happened earlier than it did.
    let step = move |what: &str| {
        use std::io::Write;
        println!("  [{:>7.0} ms] {what}", t0.elapsed().as_secs_f64() * 1000.0);
        let _ = std::io::stdout().flush();
    };

    let (handle_tx, handle_rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let processing = Arc::new(AtomicBool::new(false));

    if main_load {
        main_thread_load(&paths, &step, no_proc, stop, processing);
        hard_exit(0);
    }

    // ---- the "audio thread": load the plugin here, then process forever, exactly like engine.rs.
    let audio = {
        let (stop, processing) = (Arc::clone(&stop), Arc::clone(&processing));
        let paths = paths.clone();
        std::thread::spawn(move || {
            for path in &paths {
                let Ok(module) = Module::open(path) else { continue };
                for info in module.classes() {
                    if !info.category.contains("Audio Module") || !info.is_instrument() {
                        continue;
                    }
                    let mut plugin = match module.instantiate(&info, 44100.0, 512) {
                        Ok(p) => p,
                        Err(e) => {
                            println!("  could not start {}: {e}", info.name);
                            continue;
                        }
                    };
                    let editor = plugin.take_editor();
                    let _ = handle_tx.send((info.name.clone(), format!("{:?}", info.format), editor));

                    // Keep the callback running while the editor is opened on the other thread.
                    // **This is the part `editwin` never did**, and a VST2 plugin whose dispatcher
                    // and processReplacing are not mutually thread-safe will show it here.
                    let events = HostEvents::new();
                    let mut out: Vec<Vec<f32>> =
                        (0..plugin.out_channels.max(1)).map(|_| vec![0.0; 256]).collect();
                    processing.store(true, Ordering::Release);
                    while !stop.load(Ordering::Acquire) {
                        if !no_proc {
                            let _ = plugin.process(&events, &mut out, 256);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    processing.store(false, Ordering::Release);
                    return;
                }
            }
            let _ = handle_tx.send((String::new(), String::new(), None));
        })
    };

    let Ok((name, format, handle)) = handle_rx.recv() else {
        eprintln!("the audio thread died before handing anything over");
        hard_exit(1);
    };
    if name.is_empty() {
        eprintln!("no instrument matching `{want}`");
        hard_exit(1);
    }
    step(&format!("loaded {name} [{format}] on the audio thread"));

    let Some(handle) = handle else {
        step("no editor half — nothing to open");
        stop.store(true, Ordering::Release);
        let _ = audio.join();
        hard_exit(0);
    };

    // Let the audio thread get into its process loop first, so the editor really is opened against
    // a plugin that is being called concurrently — which is the app's actual situation.
    while !processing.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    step(if no_proc {
        "audio thread is alive but NOT processing; opening the editor on THIS (main) thread now"
    } else {
        "audio thread is processing; opening the editor on THIS (main) thread now"
    });
    step("  >>> if this is the last line, the editor open is what hangs <<<");

    let ed = match Editor::open(handle) {
        Ok(ed) => ed,
        Err(e) => {
            step(&format!("editor refused: {e}"));
            stop.store(true, Ordering::Release);
            let _ = audio.join();
            hard_exit(0);
        }
    };
    let (w, h) = ed.size();
    step(&format!("editor attached, window {w}×{h} (still not shown)"));

    // Pump for a few seconds. A VST2 editor's idle timer fires in here, so this is also where a
    // slow or re-entrant idle shows up — and the elapsed times say whether the pump kept up.
    let pump_start = Instant::now();
    let mut ticks = 0u32;
    while pump_start.elapsed().as_secs_f64() < 3.0 {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
                ticks += 1;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    step(&format!("pumped {ticks} messages over 3 s — the message loop stayed responsive"));

    step("tearing the editor down");
    drop(ed);
    step("editor torn down cleanly");

    stop.store(true, Ordering::Release);
    let _ = audio.join();
    step("audio thread stopped");

    hard_exit(0);
}
