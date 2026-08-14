//! Load a list of plugins in order, on one thread with a message pump — the plugin thread's job,
//! reproduced exactly.
//!
//!   cargo run --release --example loadseq -- "C:\path\one.dll" "C:\path\two.vst3" ...
//!
//! **Why.** Pressing Play on a song loads every plugin it uses, in instrument order, back to back
//! on the plugin thread. That is a sequence no other harness here performs: `editwin` loads each
//! plugin in isolation and `editthread` loads exactly one. If a plugin misbehaves only when it is
//! first, or only after another plugin has already claimed something, this is what shows it.
//!
//! It matters more than it sounds, because the plugin scan is CACHED: with a warm cache the app
//! opens no plugin at startup at all, so the first plugin the process ever loads is the first one
//! in the song — and Sylenth1 is known to fall over in that position (see CLAUDE.md).
//!
//! Every step is flushed, so a run that stops printing names the plugin that killed it.

use std::io::Write;
use std::time::Instant;

use volt_vsthost::{editor::Editor, hard_exit, Module};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: loadseq <plugin path> [more paths…]");
        std::process::exit(1);
    }

    let t0 = Instant::now();
    let step = |what: &str| {
        println!("[{:>7.0} ms] {what}", t0.elapsed().as_secs_f64() * 1000.0);
        let _ = std::io::stdout().flush();
    };

    // Same preparation the plugin thread does before it touches anything.
    volt_vsthost::editor::init_thread();

    // **Editors and plugins are kept apart so they can be destroyed in the right order**, which is
    // editors FIRST. A plugin's view is drawn by code inside the plugin and holds a reference to
    // its controller; terminate the plugin first and the editor's own teardown then calls into a
    // dead object. Storing them in one tuple gets this exactly backwards — Rust drops a tuple's
    // fields left to right — and the first version of this harness did, and segfaulted on three
    // plugins that were all perfectly healthy. The app has always had the order right (the plugin
    // thread drops the editor on `forget`, and the audio thread only then retires the plugin);
    // this harness had to be taught it.
    let mut plugins = Vec::new();
    let mut editors = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        step(&format!("{}/{} opening {p}", i + 1, paths.len()));
        let module = match Module::open(std::path::Path::new(p)) {
            Ok(m) => m,
            Err(e) => {
                step(&format!("    could not open: {e}"));
                continue;
            }
        };
        let Some(info) = module
            .classes()
            .into_iter()
            .find(|c| c.category.contains("Audio Module") && c.is_instrument())
        else {
            step("    no instrument class in it");
            continue;
        };

        step(&format!("    instantiating {}", info.name));
        let mut plugin = match module.instantiate(&info, 44100.0, 512) {
            Ok(p) => p,
            Err(e) => {
                step(&format!("    could not start: {e}"));
                continue;
            }
        };
        step(&format!("    started {} ({:?})", info.name, info.format));

        // And its window, because that is the other half of what the app does with it.
        if let Some(handle) = plugin.take_editor() {
            step("    opening its editor (not shown)");
            match Editor::open(handle) {
                Ok(ed) => {
                    let (w, h) = ed.size();
                    step(&format!("    editor {w}×{h}"));
                    editors.push(ed);
                }
                Err(e) => step(&format!("    no editor: {e}")),
            }
        }
        plugins.push(plugin);
        volt_vsthost::editor::pump_messages();
    }

    step(&format!("ALL {} LOADED AND ALIVE TOGETHER ({} with windows)", plugins.len(), editors.len()));

    // Hold them all at once for a moment, pumping — which is the state the app is in while a song
    // plays. A plugin that only misbehaves in company shows up here rather than in isolation.
    let hold = Instant::now();
    while hold.elapsed().as_secs_f64() < 2.0 {
        volt_vsthost::editor::pump_messages();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    step("survived two seconds together");

    step("tearing down — editors first, then the plugins they came from");
    drop(editors);
    step("  editors gone");
    drop(plugins);
    step("TORN DOWN CLEANLY");
    hard_exit(0);
}
