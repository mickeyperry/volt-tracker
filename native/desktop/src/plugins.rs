//! The plugin thread — one thread that owns every plugin's life, death and window.
//!
//! **Why this exists, in one sentence:** a VST2 plugin must be created, destroyed and shown on one
//! thread that has a Windows message pump, and neither of the two threads VOLT already had is
//! allowed to be that thread.
//!
//! The long version is the history, because both wrong answers were shipped:
//!
//! * **On the audio thread** (until 2026-08-11): a VST2 treats its creating thread as its UI
//!   thread, and the audio thread has no message pump — so opening Sytrus's window blocked inside
//!   `effEditOpen` forever and took the whole app with it.
//! * **On the main thread** (2026-08-11, morning): correct for VST2, and it froze the window
//!   instead. Instantiating a plugin is slow — Kontakt is about a second — and a song that loads
//!   four of them holds the UI thread for four seconds, which Windows paints as "not responding".
//!   That is what Mickey hit: *"the daw is just not responding"*.
//! * **Here**: a thread of VOLT's own, doing nothing but plugins, running its own message pump.
//!   Slow plugins block only this thread; the window keeps painting and the audio keeps playing.
//!
//! This is also what the engineering bible asks for — `01_ENGINE_ARCHITECTURE` puts the plugin
//! host's instance manager and GUI bridge in the control domain, owning "module lifetime, creation,
//! bus negotiation, prepare/activate/reset/destroy", with the RT side doing `process` and nothing
//! else. It took two wrong threads to arrive at the answer the document already had.
//!
//! Everything below is `Send` across the channel and then never shared: one move, one owner.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use volt_vsthost::{Editor, EditorHandle, Plugin};

/// What the plugin thread can be asked to do. Every variant that has an answer carries its own
/// reply channel, because the caller is a Tauri command that has to tell the user what happened.
pub enum PCmd {
    /// Instantiate a plugin. The slow one — seconds, for a big sampler.
    Create {
        sample_rate: f64,
        path: String,
        index: i32,
        /// What the user picked it by. A shell plugin is addressed by class INDEX and WaveShell
        /// renumbers itself between enumerations, so the index alone is not a reliable name for
        /// a plugin — see `create`.
        name: String,
        /// The slot's saved patch, applied HERE on the plugin thread — never by the caller.
        patch: Option<Vec<u8>>,
        reply: Sender<Result<(Plugin, Option<EditorHandle>), String>>,
    },
    /// Remember this slot's editor half, ready to be opened later.
    Adopt { slot: usize, handle: EditorHandle },
    /// Open (or raise) a slot's plugin window.
    EditorOpen { slot: usize, reply: Sender<Result<(), String>> },
    /// Hide it. Never destroys — one view per plugin for its whole life.
    EditorClose { slot: usize },
    /// Forget a slot entirely: its window is destroyed here, on the thread that made it.
    Forget { slot: usize },
}

/// One slot's window, in whichever state it has reached.
enum Slot {
    /// Loaded, never opened.
    Ready(EditorHandle),
    /// Open, or open-and-hidden.
    Open(Editor),
    /// Asked and refused — a plugin with no editor. Remembered so the answer is instant and the
    /// same the second time, rather than re-asking a plugin that already said no.
    None(String),
}

struct Chans {
    cmd: Sender<PCmd>,
    /// Where the audio thread posts plugins it has finished with, to be destroyed here — on the
    /// thread that created them. See `volt_engine::vst::Retire`.
    retire: Sender<Plugin>,
}

static CHANS: OnceLock<Mutex<Chans>> = OnceLock::new();

/// Start the plugin thread if it is not already running, and hand back its channels.
fn chans() -> &'static Mutex<Chans> {
    CHANS.get_or_init(|| {
        let (cmd_tx, cmd_rx) = channel::<PCmd>();
        let (retire_tx, retire_rx) = channel::<Plugin>();
        std::thread::Builder::new()
            .name("volt-plugins".into())
            .spawn(move || run(cmd_rx, retire_rx))
            .expect("could not start the plugin thread");
        Mutex::new(Chans { cmd: cmd_tx, retire: retire_tx })
    })
}

/// Post a command. Errors only if the thread has died, which it does not.
fn send(cmd: PCmd) -> Result<(), String> {
    chans()
        .lock()
        .map_err(|_| "plugin thread lock poisoned".to_string())?
        .cmd
        .send(cmd)
        .map_err(|_| "the plugin thread has stopped".to_string())
}

/// The sender the audio thread uses to hand finished plugins back for destruction.
pub fn retire_sender() -> Sender<Plugin> {
    chans().lock().unwrap_or_else(|e| e.into_inner()).retire.clone()
}

/// The thread body: pump, serve, sleep.
///
/// The order matters. Messages are pumped **first and after every command**, so a plugin that
/// opened a window during a command gets its paint messages immediately rather than 4 ms later,
/// and a long run of commands cannot starve the windows already on screen.
fn run(cmd: Receiver<PCmd>, retire: Receiver<Plugin>) {
    // Plugin editors draw with Windows controls that assume an OLE-initialised thread; Sylenth1
    // access-violates without it. Do it once, here, before any plugin is touched.
    volt_vsthost::editor::init_thread();

    // If the last session died mid-load, the plugin it was loading is now blacklisted and will not
    // be tried again. Done here rather than at app start so it happens exactly once, on the thread
    // that owns the question.
    blame_last_load();

    let mut slots: std::collections::HashMap<usize, Slot> = Default::default();

    loop {
        volt_vsthost::editor::pump_messages();

        // **Ctrl+Delete inside a plugin's own window means "take this plugin off".**
        //
        // The pump only knows it happened and to WHICH WINDOW; this thread is the only place that
        // knows which slot that window belongs to, and the page is the only place that can take the
        // plugin out of the song (the song is the truth — the engine reconciles from it). So the
        // window is hidden here for an instant answer, and the slot is sent up for the page to act
        // on. Slots above FX_EDITOR_BASE are chain effects; the page decodes that, as it already
        // does to address them.
        for id in volt_vsthost::editor::take_delete_requests() {
            let Some(slot) = slots
                .iter()
                .find(|(_, s)| matches!(s, Slot::Open(ed) if ed.window_id() == id))
                .map(|(slot, _)| *slot)
            else {
                continue;
            };
            if let Some(Slot::Open(ed)) = slots.get(&slot) {
                ed.hide();
            }
            crate::log!("plugin window: Ctrl+Delete on slot {slot} — asking the page to remove it");
            if let Some(app) = crate::APP.get() {
                use tauri::Emitter;
                let _ = app.emit("volt:vst-del", slot as u64);
            }
        }

        // Alt+] / Alt+[ inside a plugin window: walk to the next effect on that channel. Same
        // division of labour as Ctrl+Delete — this thread turns the window into a slot, and the
        // page (which owns the song, and so knows what a channel's chain is) does the walking.
        for (id, dir) in volt_vsthost::editor::take_cycle_requests() {
            let Some(slot) = slots
                .iter()
                .find(|(_, s)| matches!(s, Slot::Open(ed) if ed.window_id() == id))
                .map(|(slot, _)| *slot)
            else {
                continue;
            };
            if let Some(app) = crate::APP.get() {
                use tauri::Emitter;
                let _ = app.emit("volt:vst-cycle", (slot as u64, dir));
            }
        }

        // Space in a plugin window = the transport, exactly as in VOLT's own window. The count
        // matters only as "odd or even", but forwarding each press keeps the page's logic simple.
        let spaces = volt_vsthost::editor::take_space_requests();
        if spaces > 0 {
            if let Some(app) = crate::APP.get() {
                use tauri::Emitter;
                for _ in 0..spaces.min(4) {
                    let _ = app.emit("volt:vst-space", ());
                }
            }
        }

        // Destroy anything the audio thread has finished with. Cheap when empty, which is almost
        // always — but a plugin sitting here is holding its whole sample library in memory, so it
        // is drained every pass rather than on some slower schedule.
        while let Ok(p) = retire.try_recv() {
            drop(p);
        }

        loop {
            match cmd.try_recv() {
                Ok(c) => {
                    serve(c, &mut slots);
                    volt_vsthost::editor::pump_messages();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // Every sender is gone, which only happens at shutdown. Nothing left to serve.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // 4 ms is a ~250 Hz pump: far finer than the 30 ms VST2 idle tick, and small enough that a
        // click on a plugin window never feels late. It costs nothing measurable when idle.
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
}

fn serve(c: PCmd, slots: &mut std::collections::HashMap<usize, Slot>) {
    match c {
        PCmd::Create { sample_rate, path, index, name, patch, reply } => {
            let _ = reply.send(create(sample_rate, &path, index, &name, patch));
        }
        PCmd::Adopt { slot, handle } => {
            crate::log!("adopt: slot {slot} now offers the window for {}", handle.name);
            // Replacing a slot's plugin destroys the old window here, before the new one arrives.
            slots.remove(&slot);
            slots.insert(slot, Slot::Ready(handle));
        }
        PCmd::Forget { slot } => {
            crate::log!("forget: slot {slot} dropped (window destroyed if it had one)");
            slots.remove(&slot);
        }
        PCmd::EditorClose { slot } => {
            if let Some(Slot::Open(ed)) = slots.get(&slot) {
                ed.hide();
            }
        }
        PCmd::EditorOpen { slot, reply } => {
            let answer = match slots.remove(&slot) {
                None => Err("no plugin on this instrument".to_string()),
                Some(Slot::None(name)) => {
                    let msg = format!("{name} has no window of its own");
                    slots.insert(slot, Slot::None(name));
                    Err(msg)
                }
                Some(Slot::Open(ed)) => {
                    // Already open, possibly behind VOLT: raise it. Never a second view.
                    ed.show();
                    slots.insert(slot, Slot::Open(ed));
                    Ok(())
                }
                Some(Slot::Ready(handle)) => {
                    let name = handle.name.clone();
                    match Editor::open(handle) {
                        Ok(ed) => {
                            ed.show();
                            slots.insert(slot, Slot::Open(ed));
                            Ok(())
                        }
                        Err(e) => {
                            // The handle is consumed either way, so a plugin that refuses is
                            // remembered as having none rather than re-asked on every click.
                            slots.insert(slot, Slot::None(name));
                            Err(e)
                        }
                    }
                }
            };
            match &answer {
                Ok(()) => crate::log!("editor: slot {slot} window is up"),
                Err(e) => crate::log!("editor: slot {slot} REFUSED: {e}"),
            }
            let _ = reply.send(answer);
        }
    }
}

/// The plugin currently being loaded, written before the attempt and erased after it.
///
/// **This is the only protection against a plugin that hangs while loading.** A wedged plugin
/// thread cannot be rescued — Rust cannot kill a thread, and the plugin is somewhere inside its own
/// `LoadLibrary` — so the app has to be closed, and without this the next launch would load the
/// same plugin and wedge again, for ever. `Bionic Delay` does exactly this. On startup a leftover
/// entry means "this one killed us last time": it goes on the blacklist and is never opened again.
///
/// Same idea as the scan journal in `engine.rs`, for the same reason, arrived at the same way.
fn load_journal() -> Option<std::path::PathBuf> {
    crate::log::volt_dir().map(|d| d.join("vst-loading.txt"))
}

/// The deliberate-quit marker, beside the journal. Every intended exit writes it (`app_quit`,
/// and the ten-second escape hatch); a crash or a kill never does. It is what lets the journal
/// tell "this plugin wedged us" apart from "the app was closed while this plugin was loading" —
/// without it, closing VOLT for a rebuild while a song's plugins were still coming up
/// blacklisted two perfectly innocent plugins (Sytrus and WaveShell, 2026-08-23).
fn clean_exit_marker() -> Option<std::path::PathBuf> {
    crate::log::volt_dir().map(|d| d.join("clean-exit.txt"))
}

/// Called from every deliberate exit path, right before `hard_exit`.
pub fn note_clean_exit() {
    if let Some(p) = clean_exit_marker() {
        let _ = std::fs::write(&p, b"1");
    }
}

/// If a previous run died while loading a plugin, blacklist it. Returns its path, for the log.
///
/// Unless that run ENDED ON PURPOSE while the load was still young: the marker above carries the
/// quit's mtime, the journal carries the load's, and a load under 25 s old at a deliberate quit
/// is an interrupted load, not a wedge (the app's own answer timeout is 30 s — a load the user
/// sat through longer than that before quitting had genuinely hung, and is still blamed). A
/// STALE marker cannot acquit a NEWER journal: the quit would predate the load and the duration
/// test fails closed, toward blaming.
fn blame_last_load() -> Option<String> {
    let jp = load_journal()?;
    let started = std::fs::metadata(&jp).ok().and_then(|md| md.modified().ok());
    let stuck = std::fs::read_to_string(&jp).ok()?.trim().to_string();
    let _ = std::fs::remove_file(&jp);
    let quit_at = clean_exit_marker().and_then(|m| {
        let t = std::fs::metadata(&m).ok().and_then(|md| md.modified().ok());
        let _ = std::fs::remove_file(&m);
        t
    });
    if stuck.is_empty() {
        return None;
    }
    if !journal_blames(started, quit_at) {
        eprintln!("plugins: {stuck} was loading when the app was closed on purpose — not blamed");
        return None;
    }
    if volt_vsthost::blacklist(std::path::Path::new(&stuck)) {
        eprintln!("plugins: {stuck} killed the last session while loading — blacklisted");
    }
    Some(stuck)
}

/// The verdict alone, so it can be tested: blame unless a deliberate quit interrupted a load
/// under 25 s old. Missing evidence on either side means blame — the old behaviour.
fn journal_blames(
    started: Option<std::time::SystemTime>,
    quit_at: Option<std::time::SystemTime>,
) -> bool {
    match (started, quit_at) {
        (Some(s), Some(q)) => match q.duration_since(s) {
            Ok(d) => d.as_secs() >= 25,
            Err(_) => true, // the "quit" predates the load — a stale marker acquits nothing
        },
        _ => true,
    }
}

/// Instantiate one plugin and take its editor half. **The slow call**, and the reason this thread
/// exists rather than the work being done on the main one.
fn create(
    sample_rate: f64,
    path: &str,
    index: i32,
    name: &str,
    patch: Option<Vec<u8>>,
) -> Result<(Plugin, Option<EditorHandle>), String> {
    let p = std::path::Path::new(path);

    // Excluded by decision, not by accident — and refused HERE as well as in the scan, because a
    // song saved before the exclusion (or one carried from another machine) would otherwise still
    // open it. See `SCAN_SKIP`.
    if volt_vsthost::skip_in_scan(p) {
        return Err(format!(
            "{} is not hosted by VOLT on purpose — it is a whole DAW wrapped as a plugin",
            p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        ));
    }

    // A plugin known to take the process down is not tried again, and says why rather than
    // failing mysteriously. Deleting its line from the file gives it another chance — so the
    // message names the file by its real path rather than reciting a Windows environment variable.
    if volt_vsthost::is_blacklisted(p) {
        let file = volt_vsthost::blacklist_path()
            .map(|b| b.display().to_string())
            .unwrap_or_else(|| "the blacklist file".into());
        return Err(format!(
            "{} is blacklisted — it crashed or hung VOLT before. Remove its line from \
             {file} to try it again.",
            p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| path.into())
        ));
    }

    // Write who we are about to open, and FLUSH — an unflushed line dies with the process, which
    // would defeat the entire point.
    if let Some(jp) = load_journal() {
        if let Some(dir) = jp.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::File::create(&jp) {
            let _ = write!(f, "{path}");
            let _ = f.flush();
        }
    }
    let done = || {
        if let Some(jp) = load_journal() {
            let _ = std::fs::remove_file(jp);
        }
    };

    crate::log!("create: opening {path} (class {index})");
    let module = volt_vsthost::Module::open(p)
        .inspect_err(|e| { done(); crate::log!("create: open FAILED: {e}"); })?;
    let classes = module.classes();
    // **A shell plugin's class INDEX is not a stable name for it.** One `.vst3` file can hold
    // hundreds of plugins — WaveShell holds hundreds of Waves effects — addressed by index, and
    // that file does not enumerate the same way twice: the same shell answered with 861 classes
    // and then 609, so a song holding "index 864" got "no class 864 in WaveShell…" and RBass
    // simply refused to load (2026-08-15). The NAME the user picked survives renumbering, so it
    // is the fallback, restricted to audio classes because a VST3 file also exports a controller
    // class under the same name and instancing that one returns E_NOINTERFACE.
    let by_index = classes.iter().find(|c| c.index == index);
    let info = match by_index {
        Some(c) => c,
        None => {
            let alt = classes
                .iter()
                .find(|c| c.name == name && c.category.contains("Audio Module"));
            match alt {
                Some(c) => {
                    crate::log!(
                        "create: class {index} is gone from {path} — found “{name}” at {} instead \
                         (a shell renumbered itself)",
                        c.index
                    );
                    c
                }
                None => {
                    done();
                    return Err(format!(
                        "no class {index} in {path} — and nothing in it is called “{name}” \
                         ({} classes offered). Re-pick the plugin: shell plugins renumber.",
                        classes.len()
                    ));
                }
            }
        }
    };
    // 512 is a ceiling, not the block size — the render quantum is 128 and a plugin told to expect
    // at most 512 is happy with less.
    let mut plugin = module
        .instantiate(info, sample_rate, 512)
        .inspect_err(|e| { done(); crate::log!("create: instantiate FAILED: {e}"); })?;
    // **The saved patch goes in HERE, on the plugin thread, before anyone else can touch it.**
    //
    // It used to be applied by the caller — `fxvst_load`, running on a Tauri worker — and that
    // crashed VOLT: apQualizr2 entered a critical section belonging to an instance that had just
    // been built on a different thread and took an access violation inside ntdll
    // (0xc0000005, `RtlEnterCriticalSection` from apQualizr2, dump 2026-08-15 01:49). It is the
    // same rule the rest of this file is built on and the same one PARAM_LISTS follows: a plugin
    // belongs to the thread that made it until it is deliberately handed on. `setState` is not an
    // exception just because it looks like data.
    if let Some(bytes) = patch {
        match plugin.set_state(&bytes) {
            Ok(()) => crate::log!("create: patch restored ({} bytes)", bytes.len()),
            Err(e) => crate::log!("create: the plugin refused its saved patch: {e}"),
        }
    }
    let editor = plugin.take_editor();
    // Survived. Whatever happens later, it was not this plugin's loading that did it.
    done();
    crate::log!("create: {} ready (editor half: {})", plugin.name,
                if editor.is_some() { "yes" } else { "NONE - no window will be offered" });
    Ok((plugin, editor))
}

// ---- the patch cache -------------------------------------------------------------------------

/// Every plugin's last known patch, by owner: `"i7"` for instrument slot 7, `"f0.1"` for the
/// second effect on track 0. The value carries the plugin's identity (`path|index`) alongside the
/// bytes, so a slot that changed plugin can never be handed the previous one's state.
///
/// **This is what stops a preset evaporating.** A plugin instance does not survive a device
/// reopen — the audio thread and everything in it is replaced — so before the old engine goes
/// away its plugins are asked for their state, and a freshly created instance is given it back
/// before it plays a note. Without this, changing the buffer size, choosing another interface or
/// the engine healing itself all silently reverted every plugin to its factory preset, which is
/// exactly what Mickey saw: *"i launch sytrus, put some preset there and after some time it reset
/// to the default preset"*.
///
/// In memory only, for now: this survives a reopen, not a restart. Putting patches in the SONG is
/// the next piece, and it needs a size story first — a Kontakt state is megabytes and the autosave
/// goes to localStorage, which must never be the thing that breaks saving.
static PATCHES: Mutex<Option<std::collections::HashMap<String, (String, Vec<u8>)>>> =
    Mutex::new(None);

/// Which plugin file each owner currently holds, so the audio thread — which knows its slots but
/// not what was loaded into them — can key a patch correctly on its way out.
static IDENTS: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);

/// Record what a slot was loaded with, at load time.
pub fn remember_ident(key: String, ident: String) {
    let mut g = IDENTS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(key, ident);
}

/// What this slot holds, if anything.
pub fn ident_for(key: &str) -> Option<String> {
    let g = IDENTS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref()?.get(key).cloned()
}

/// Remember a plugin's patch under its owner key.
pub fn remember_patch(key: String, ident: String, bytes: Vec<u8>) {
    let mut g = PATCHES.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(key, (ident, bytes));
}

/// The patch saved for this owner, if it belongs to the same plugin.
pub fn patch_for(key: &str, ident: &str) -> Option<Vec<u8>> {
    let g = PATCHES.lock().unwrap_or_else(|e| e.into_inner());
    let (saved_ident, bytes) = g.as_ref()?.get(key)?;
    (saved_ident == ident).then(|| bytes.clone())
}

// ---- keeping a chain's patches when the chain itself changes shape --------------------------
//
// **A patch belongs to a PLUGIN, but it is filed under a POSITION.** Take link 1 off a channel and
// every plugin behind it slides down a slot: the one that was at `f10.2` is now `f10.1`, looks up
// the patch filed at `f10.1`, finds the *removed* plugin's (wrong identity, so: nothing) and comes
// back at its factory preset. Mickey, having deleted IL Delay from channel 11: *"another vst got
// initiated with its default preset"* (2026-08-15).
//
// So the patches are moved with the plugins. The caller snapshots the live states first
// (`vst_patch_flush`), then calls one of these, then lets the reconciler rebuild the chain — and
// every link finds its own patch waiting under its new key. IDENTS moves in step with PATCHES,
// because it is what tells "this is the same plugin" from "this is somebody else's blob".

/// Renumber one chain after a link is removed: the removed one is dropped, everything above it
/// slides down by one.
pub fn patches_after_remove(track: i64, pos: usize) {
    remap_chain(track, &|p| if p == pos { None } else if p > pos { Some(p - 1) } else { Some(p) });
}

/// Two links swapped places (the ▲▼ reorder) — their patches swap with them.
pub fn patches_after_swap(track: i64, a: usize, b: usize) {
    remap_chain(track, &|p| {
        Some(if p == a {
            b
        } else if p == b {
            a
        } else {
            p
        })
    });
}

/// The shared move: rebuild every `f{track}.{pos}` key in both maps through `map`. Generic over
/// the value because the two maps hold different things and both have to move in step — a patch
/// that outlives its identity entry is a patch that will be refused.
fn repos_keys<V>(
    m: &mut std::collections::HashMap<String, V>,
    prefix: &str,
    map: &dyn Fn(usize) -> Option<usize>,
) {
    // Pull the chain's entries out, then put them back under their new names. Taken out first so
    // a move onto a key that is itself moving cannot clobber it.
    let keys: Vec<(usize, String)> = m
        .keys()
        .filter(|k| k.starts_with(prefix))
        .filter_map(|k| k[prefix.len()..].parse::<usize>().ok().map(|p| (p, k.clone())))
        .collect();
    let taken: Vec<(usize, V)> =
        keys.into_iter().filter_map(|(p, k)| m.remove(&k).map(|v| (p, v))).collect();
    for (p, v) in taken {
        if let Some(np) = map(p) {
            m.insert(format!("{prefix}{np}"), v);
        }
    }
}

fn remap_chain(track: i64, map: &dyn Fn(usize) -> Option<usize>) {
    let prefix = format!("f{track}.");
    if let Some(m) = PATCHES.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        repos_keys(m, &prefix, map);
    }
    if let Some(m) = IDENTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        repos_keys(m, &prefix, map);
    }
    crate::log!("patches: track {track} chain renumbered");
}

// (There is deliberately no `drop_patch` any more. Retiring a patch on unload was the bug that
// reset every plugin in a chain when one was moved — see `vst_unload`. The only place a patch is
// genuinely retired is `patches_after_remove`, which says so.)

// ---- patches on DISK — the restart half of "the preset must not evaporate" -------------------
//
// PATCHES survives a device reopen; this survives closing VOLT. One file PER SONG at
// `<data dir>/patches/<song-id>.pat` (`%LOCALAPPDATA%\VOLT\patches` on Windows — see
// `log::volt_dir`), because the same slot in two songs holds two
// different presets — a global store would hand song B whatever song A left in slot i7 the
// moment both used the same plugin there. The UI names the song (`vst_patch_song`) before any
// plugin of it loads; the file's entries land in PATCHES, and `vst_load`'s existing
// patch-at-create path does the rest. Written on every transport stop and on the way out of
// every device close and quit — so a crash costs at most the tweaks since the last stop.
//
// Deliberately NOT in the song file: a Kontakt state is megabytes, the autosave goes to
// localStorage, and nothing is ever allowed to be the thing that breaks saving.
//
// The format is length-prefixed binary (magic, then key/ident/blob triples), not JSON — the
// blobs are opaque megabyte byte-strings and base64-in-JSON would only make them bigger and
// slower for zero readability where it counts.
static PATCH_SONG: Mutex<Option<String>> = Mutex::new(None);

fn patches_path(song: &str) -> Option<std::path::PathBuf> {
    let dir = crate::log::volt_dir()?;
    // song ids are ours (newId()), but a filename must never trust its input
    let safe: String = song
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    Some(dir.join("patches").join(format!("{safe}.pat")))
}

const PATCH_MAGIC: &[u8; 8] = b"VOLTPAT1";

/// Write the in-memory patch cache to the current song's file. Atomic (tmp + rename), so a
/// crash mid-write leaves the previous good file, never half of one. No song named yet → no-op:
/// there is nothing to key the file by.
pub fn flush_patches() {
    let song = PATCH_SONG.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(song) = song else { return };
    let Some(path) = patches_path(&song) else { return };
    let map = {
        let g = PATCHES.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref().cloned().unwrap_or_default()
    };
    let mut out: Vec<u8> = Vec::with_capacity(64);
    out.extend_from_slice(PATCH_MAGIC);
    for (key, (ident, bytes)) in &map {
        for part in [key.as_bytes(), ident.as_bytes(), bytes.as_slice()] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("pat.tmp");
    if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
        crate::log!("patches: {} entries ({} bytes) → {}", map.len(), out.len(), path.display());
    }
}

fn read_patches(path: &std::path::Path) -> Option<std::collections::HashMap<String, (String, Vec<u8>)>> {
    let data = std::fs::read(path).ok()?;
    let rest = data.strip_prefix(PATCH_MAGIC.as_slice())?;
    let mut map = std::collections::HashMap::new();
    let mut p = 0usize;
    // tolerant of truncation: keep every complete entry, drop the torn tail
    let take = |p: &mut usize| -> Option<Vec<u8>> {
        let n = u32::from_le_bytes(rest.get(*p..*p + 4)?.try_into().ok()?) as usize;
        *p += 4;
        let v = rest.get(*p..*p + n)?.to_vec();
        *p += n;
        Some(v)
    };
    while p < rest.len() {
        let Some(key) = take(&mut p) else { break };
        let Some(ident) = take(&mut p) else { break };
        let Some(bytes) = take(&mut p) else { break };
        if let (Ok(key), Ok(ident)) = (String::from_utf8(key), String::from_utf8(ident)) {
            map.insert(key, (ident, bytes));
        }
    }
    Some(map)
}

/// Point the patch store at a song: flush the outgoing song's cache to its file, then REPLACE
/// the cache with the named song's saved patches (replace, not merge — one song's presets must
/// never bleed into another's). Idempotent per id, so the UI can call it as often as it likes.
pub fn set_song(id: String) {
    {
        let g = PATCH_SONG.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_deref() == Some(id.as_str()) {
            return;
        }
    }
    flush_patches();
    let loaded = patches_path(&id).and_then(|p| read_patches(&p)).unwrap_or_default();
    crate::log!("patches: song {} — {} saved patch(es) on disk", id, loaded.len());
    {
        let mut g = PATCHES.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some(loaded);
    }
    let mut g = PATCH_SONG.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(id);
}

// ---- what the Tauri commands call ------------------------------------------------------------

/// Create a plugin on the plugin thread and wait for it. Blocks the calling Tauri worker, never
/// the UI.
pub fn create_plugin(
    sample_rate: f64,
    path: String,
    index: i32,
    name: String,
    patch: Option<Vec<u8>>,
) -> Result<(Plugin, Option<EditorHandle>), String> {
    let (reply, answer) = channel();
    send(PCmd::Create { sample_rate, path, index, name, patch, reply })?;
    // Generous, because a big sampler really is slow — but bounded, because a plugin that never
    // returns must not leave the caller waiting for ever.
    answer
        .recv_timeout(std::time::Duration::from_secs(60))
        .map_err(|_| "the plugin did not finish loading".to_string())?
}

pub fn adopt_editor(slot: usize, handle: EditorHandle) {
    let _ = send(PCmd::Adopt { slot, handle });
}

pub fn forget(slot: usize) {
    let _ = send(PCmd::Forget { slot });
}

pub fn editor_open(slot: usize) -> Result<(), String> {
    let (reply, answer) = channel();
    send(PCmd::EditorOpen { slot, reply })?;
    answer
        .recv_timeout(std::time::Duration::from_secs(60))
        .map_err(|_| "the plugin did not finish opening its window".to_string())?
}

pub fn editor_close(slot: usize) {
    let _ = send(PCmd::EditorClose { slot });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A deliberate quit does not turn an innocent plugin into a blacklisted one.**
    ///
    /// The journal alone cannot tell "this plugin wedged us" from "the app was closed while this
    /// plugin was loading" — Sytrus and WaveShell were wrongly banned exactly that way
    /// (2026-08-23). The verdict: blame on a crash (no marker), blame when the load had already
    /// hung longer than the app's own patience before the quit, acquit a young load a quit
    /// interrupted, and never let a STALE marker acquit a newer journal.
    #[test]
    fn a_deliberate_quit_acquits_a_young_load_and_nothing_else() {
        use std::time::{Duration, SystemTime};
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let plus = |s: u64| Some(t0 + Duration::from_secs(s));
        // crash: no marker → blame
        assert!(journal_blames(plus(0), None));
        // clean quit 3 s into the load → interrupted, not wedged
        assert!(!journal_blames(plus(0), plus(3)));
        // clean quit after sitting on a stuck load for 40 s → genuinely wedged, still blamed
        assert!(journal_blames(plus(0), plus(40)));
        // a stale marker from long before this load acquits nothing
        assert!(journal_blames(plus(100), plus(0)));
        // no journal mtime readable → the old behaviour: blame
        assert!(journal_blames(None, plus(3)));
    }

    /// **A patch follows its plugin when the chain is renumbered.**
    ///
    /// Patches are filed by POSITION (`f10.0`, `f10.1`, …) and positions move: take a link out and
    /// everything behind it slides down. Without moving the patches with them, the plugin that
    /// lands in slot 1 looks up slot 1's patch — the REMOVED plugin's — and, the identity check
    /// failing, comes back at its factory preset. That is exactly what deleting IL Delay from
    /// channel 11 did to its neighbour (2026-08-15). `repos_keys` is the whole move, and it is
    /// pure, so it is pinned here rather than through the globals it normally edits.
    #[test]
    fn removing_a_link_slides_the_patches_behind_it_down() {
        let mut m: std::collections::HashMap<String, &str> = Default::default();
        m.insert("f10.0".into(), "eq");
        m.insert("f10.1".into(), "delay");   // the one being removed
        m.insert("f10.2".into(), "reverb");
        m.insert("f-1.1".into(), "master limiter"); // another chain: must not move
        repos_keys(&mut m, "f10.", &|p| if p == 1 { None } else if p > 1 { Some(p - 1) } else { Some(p) });
        assert_eq!(m.get("f10.0").copied(), Some("eq"), "the link in front stays put");
        assert_eq!(m.get("f10.1").copied(), Some("reverb"), "the one behind slid down WITH its patch");
        assert!(m.get("f10.2").is_none(), "and nothing is left at the old end of the chain");
        assert!(!m.values().any(|v| *v == "delay"), "the removed link's patch is gone");
        assert_eq!(m.get("f-1.1").copied(), Some("master limiter"), "another channel is untouched");
    }

    /// The ▲▼ reorder: two links exchange places, and their patches exchange with them. A swap is
    /// the case a naive one-at-a-time rename gets wrong (it would overwrite the first move), which
    /// is why the entries are taken out of the map before any go back in.
    #[test]
    fn reordering_two_links_swaps_their_patches() {
        let mut m: std::collections::HashMap<String, &str> = Default::default();
        m.insert("f3.0".into(), "comp");
        m.insert("f3.1".into(), "sausage");
        m.insert("f3.2".into(), "eq");
        repos_keys(&mut m, "f3.", &|p| Some(if p == 0 { 1 } else if p == 1 { 0 } else { p }));
        assert_eq!(m.get("f3.0").copied(), Some("sausage"));
        assert_eq!(m.get("f3.1").copied(), Some("comp"));
        assert_eq!(m.get("f3.2").copied(), Some("eq"), "the untouched link keeps its own");
        assert_eq!(m.len(), 3, "nothing was lost or duplicated in the swap");
    }
}
