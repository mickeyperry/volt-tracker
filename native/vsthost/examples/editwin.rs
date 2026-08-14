//! Prove a plugin will actually draw into a window we make — without putting one on screen.
//!
//!   cargo run --release --example editwin -- <name fragment>   — one plugin
//!   cargo run --release --example editwin                      — every instrument on the machine
//!
//! **Why this exists.** `editor.rs` is a pile of COM call-order rules where getting one wrong
//! produces a plugin that loads, plays, and silently has no window — which is indistinguishable
//! from a plugin that simply has no editor. Nothing in the test suite can reach it: the browser
//! harness has no Win32, and driving VOLT's own UI is off the table. So this runs the whole attach
//! path against real plugins in a separate process and reports what came back.
//!
//! It never calls `show()`, so nothing appears and nothing steals focus — safe to run while Mickey
//! is working. What it proves is everything up to that point: `createView` returned a view, the
//! view accepts an HWND, `setFrame` and `attached` succeeded, and `getSize` produced a sane size.
//! A plugin whose editor is 640×480 is reporting our default and probably never sized itself; one
//! that comes back with its own numbers has genuinely laid itself out in our window.

use std::path::Path;

use volt_vsthost::{editor::Editor, hard_exit, read_blacklist, scan, scan_vst2, Module, VST3_DIR};

fn main() {
    let want = std::env::args().nth(1).map(|s| s.to_lowercase());

    let banned = read_blacklist();
    let is_banned = |p: &Path| banned.iter().any(|b| b.as_str() == p.to_string_lossy());

    let mut paths: Vec<_> = scan(Path::new(VST3_DIR));
    paths.extend(scan_vst2().dlls);
    paths.retain(|p| !is_banned(p));

    let (mut tried, mut windowed, mut none, mut failed) = (0, 0, 0, 0);

    for path in &paths {
        // Filter on the file name before opening: loading every plugin on the machine to answer
        // one search is minutes of plugin roulette.
        if let Some(w) = &want {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !stem.contains(w.as_str()) {
                continue;
            }
        }
        let Ok(module) = Module::open(path) else { continue };
        for info in module.classes() {
            // Audio classes only — a VST3 also exports its controller class under the same name,
            // and asking that one for an IComponent is an E_NOINTERFACE by design, not a fault.
            if !info.category.contains("Audio Module") || !info.is_instrument() {
                continue;
            }
            let mut plugin = match module.instantiate(&info, 44100.0, 512) {
                Ok(p) => p,
                Err(e) => {
                    println!("  FAIL  {:<28} could not start: {e}", trunc(&info.name, 27));
                    failed += 1;
                    continue;
                }
            };
            tried += 1;

            let Some(handle) = plugin.take_editor() else {
                println!("  ----  {:<28} no editor half at all", trunc(&info.name, 27));
                none += 1;
                continue;
            };

            match Editor::open(handle) {
                Ok(ed) => {
                    let (w, h) = ed.size();
                    println!("  OK    {:<28} {:?}  window {w}×{h}", trunc(&info.name, 27), info.format);
                    windowed += 1;
                    // Dropping it here exercises the teardown too — removed(), setFrame(null),
                    // release, DestroyWindow — which is where a wrong order shows up as a crash.
                    drop(ed);
                }
                Err(e) => {
                    println!("  none  {:<28} {e}", trunc(&info.name, 27));
                    none += 1;
                }
            }
        }
    }

    println!();
    println!("{tried} instruments tried: {windowed} opened a window, {none} have none, \
              {failed} would not start");

    // Every plugin on the machine is mapped into this process by now. Do not hand that to Windows'
    // DLL teardown — see hard_exit.
    hard_exit(0);
}

fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else { format!("{}…", &s[..n - 1]) }
}
