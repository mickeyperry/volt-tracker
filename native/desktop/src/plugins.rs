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
        PCmd::Create { sample_rate, path, index, patch, reply } => {
            let _ = reply.send(create(sample_rate, &path, index, patch));
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
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|d| d.join("VOLT").join("vst-loading.txt"))
}

/// If a previous run died while loading a plugin, blacklist it. Returns its path, for the log.
fn blame_last_load() -> Option<String> {
    let jp = load_journal()?;
    let stuck = std::fs::read_to_string(&jp).ok()?.trim().to_string();
    let _ = std::fs::remove_file(&jp);
    if stuck.is_empty() {
        return None;
    }
    if volt_vsthost::blacklist(std::path::Path::new(&stuck)) {
        eprintln!("plugins: {stuck} killed the last session while loading — blacklisted");
    }
    Some(stuck)
}

/// Instantiate one plugin and take its editor half. **The slow call**, and the reason this thread
/// exists rather than the work being done on the main one.
fn create(
    sample_rate: f64,
    path: &str,
    index: i32,
    patch: Option<Vec<u8>>,
) -> Result<(Plugin, Option<EditorHandle>), String> {
    let p = std::path::Path::new(path);

    // A plugin known to take the process down is not tried again, and says why rather than
    // failing mysteriously. Deleting its line from the file gives it another chance.
    if volt_vsthost::is_blacklisted(p) {
        return Err(format!(
            "{} is blacklisted — it crashed or hung VOLT before. Remove its line from \
             %LOCALAPPDATA%\\VOLT\\vst-blacklist.txt to try it again.",
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
    let Some(info) = classes.iter().find(|c| c.index == index) else {
        done();
        return Err(format!("no class {index} in {path}"));
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

/// Forget an owner's patch — the user took the plugin off that slot on purpose.
pub fn drop_patch(key: &str) {
    let mut g = PATCHES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = g.as_mut() {
        m.remove(key);
    }
}

// ---- what the Tauri commands call ------------------------------------------------------------

/// Create a plugin on the plugin thread and wait for it. Blocks the calling Tauri worker, never
/// the UI.
pub fn create_plugin(
    sample_rate: f64,
    path: String,
    index: i32,
    patch: Option<Vec<u8>>,
) -> Result<(Plugin, Option<EditorHandle>), String> {
    let (reply, answer) = channel();
    send(PCmd::Create { sample_rate, path, index, patch, reply })?;
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
