//! VOLT's plugin host — VST3 and VST2 behind one front door.
//!
//! A VST3 plugin is a DLL that exports one useful function, `GetPluginFactory`, which hands back a
//! COM object listing the classes inside it. Instantiate one of those classes, ask it for
//! `IAudioProcessor`, tell it the sample rate and block size, and then hand it buffers and note
//! events a block at a time. That is the whole shape of it; everything in `win.rs` is that
//! sequence with the error handling written down.
//!
//! VST2 (in `vst2.rs`) is the same job over an older, smaller API. The point of the [`Module`] and
//! [`Plugin`] types is that **nothing upstream knows which format it is holding**: `Module::open`
//! picks the loader from the file itself (`.dll` is VST2, everything else VST3), and both formats
//! process through the same event list and the same buffer shape. The engine's slot map,
//! `VstNode`, the desktop commands and the song's `ins.vst` all stay format-blind.
//!
//! **Why this crate is separate from `volt-engine`.** The engine is a transliteration of VOLT's own
//! instruments and is graded by null tests against the browser. A plugin is somebody else's code
//! running in our process — it can be slow, it can be wrong, and it can crash. Keeping it behind
//! its own crate boundary means the scanning tool that finds out *which* of Mickey's 129 plugins
//! misbehave is a separate binary from the thing playing his songs.
//!
//! **The platform split lives HERE and nowhere upstream.** Plugin hosting is Windows through and
//! through — LoadLibrary, COM, Win32 windows — so the real implementation (`win.rs` and its
//! submodules) only exists on Windows, and `portable.rs` presents the same public surface
//! everywhere else with honest answers: scans come back empty, `Module::open` says "VST hosting is
//! not available on this platform yet", message pumps pump nothing. The engine and the desktop
//! crates compile against either without a single `cfg` of their own — that is the whole design,
//! and it is what lets a Linux or macOS build ship the engine and every ported instrument while
//! saying plainly what it does not have.
//!
//! What is genuinely portable lives in this file and behaves identically on all three platforms:
//! the plugin *descriptions*, the scan exclusion (a naming rule, not an OS call), the blacklist,
//! the touch log, and `hard_exit`.
//!
//! **On safety.** Almost everything in the Windows half is `unsafe`: it is COM, over FFI, into
//! code we did not write. The rule followed throughout is that every raw pointer coming out of a
//! plugin is checked against null and every `tresult` is checked against `kResultOk` before
//! anything is done with it, because the alternative is a crash in someone's session with no way
//! to tell which plugin did it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ---- the platform facade -------------------------------------------------------------------
//
// One of these two is the crate, the other does not exist. Their public surfaces are kept
// identical BY HAND — there is no trait tying them together, because a trait would force dynamic
// dispatch (or generics) on the engine for a distinction that is settled at compile time. If you
// add a public item to `win.rs`, add its honest twin to `portable.rs` in the same commit, or the
// next Linux build is the thing that finds out.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
mod portable;
#[cfg(not(windows))]
pub use portable::*;

// ---- format-neutral plugin descriptions (identical on every platform) -----------------------

/// Which API a plugin speaks. Carried on [`PluginInfo`] so a caller *can* ask, and shaped so it
/// never has to: load by path, play through the same calls, and the difference stays in this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginFormat {
    Vst3,
    Vst2,
}

/// One plugin class found by a scan. A single `.vst3` file can contain several — an instrument and
/// its own effect, say — so this is per class, not per file. (A VST2 DLL is always one plugin,
/// `index` 0.)
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// The DLL to load. For a bundle this is the binary *inside* it, not the folder.
    pub path: PathBuf,
    /// Class index within the factory, which is how `load` finds it again.
    pub index: i32,
    pub name: String,
    /// `Audio Module Class` for anything that processes audio. Effects and instruments both.
    pub category: String,
    /// Where instrument-vs-effect actually lives: a VSTi says `Instrument` in here.
    pub subcategories: String,
    pub vendor: String,
    pub cid: [i8; 16],
    pub format: PluginFormat,
}

impl PluginInfo {
    /// A VSTi — something that turns notes into audio, which is what VOLT wants first.
    ///
    /// The VST3 spec has no "is an instrument" flag; it has a free-text subcategory string, by
    /// convention containing `Instrument` for a VSTi (`Instrument|Synth`, `Instrument|Sampler`).
    /// So this is a substring match, because that is genuinely what the format offers.
    pub fn is_instrument(&self) -> bool {
        self.subcategories.contains("Instrument")
    }
}

/// One automatable parameter: its id (a VST3 `ParamID`, or a VST2 parameter index) and the name
/// the plugin gives it. What the automation picker lists and what `Plugin::set_param` takes.
#[derive(Debug, Clone)]
pub struct ParamDesc {
    pub id: u32,
    pub name: String,
}

// ---- the scan exclusion ------------------------------------------------------------------------

/// Plugins a SCAN never opens, by name, because hosting them is a bad idea rather than a bug.
///
/// Distinct from the blacklist below: that one is earned, at the cost of a crashed or hung
/// session, and can be cleared by hand. This is a decision. So far it holds one thing and its
/// reasoning is the whole rule: **`FL Studio VSTi` is FL Studio itself wrapped as a plugin** —
/// loading it boots an entire DAW inside VOLT's process, with its own audio device, its own
/// windows and its own plugin host, and a scan would open it just to read its name. Mickey, who
/// owns it and knows it best: *"fl vsti might do issues pleases exclude it"* (2026-08-15).
///
/// Matched on the file name, lowercased, as a prefix — one entry covers `FL Studio VSTi.dll` and
/// `FL Studio VSTi (Multi).dll` both. An explicit open still works (`examples/openone.rs` takes a
/// path and means it); this only stops the sweep from touching it.
///
/// Shared rather than Windows-only because it is a naming rule and nothing else: no OS call, no
/// plugin loaded, just a string test that has to answer the same way wherever it is asked.
pub const SCAN_SKIP: &[&str] = &["fl studio vsti"];

/// Should the scan walk past this file? See [`SCAN_SKIP`].
pub fn skip_in_scan(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    SCAN_SKIP.iter().any(|s| name.starts_with(s))
}

// ---- where VOLT keeps its data --------------------------------------------------------------

/// The VOLT data folder, per platform: `%LOCALAPPDATA%\VOLT` on Windows, `$XDG_DATA_HOME/VOLT`
/// (falling back to `~/.local/share/VOLT`) on Linux, `~/Library/Application Support/VOLT` on
/// macOS. The blacklist below lives in it, and the desktop crate uses this same function for the
/// log, the journals, the scan cache, the saved patches and the tap — **one helper, one folder**,
/// because the blacklist this crate writes is the one the app's scan reads, and two answers to
/// "where is the VOLT folder" would eventually disagree.
pub fn volt_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library").join("Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute()) // the XDG spec says a relative value must be ignored
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")));
    Some(base?.join("VOLT"))
}

// ---- the blacklist ---------------------------------------------------------------------------

/// Where the plugin blacklist lives: `<data dir>/vst-blacklist.txt` (on Windows that is the
/// familiar `%LOCALAPPDATA%\VOLT\vst-blacklist.txt`), one full path per line, hand-editable
/// (delete a line to give a plugin another chance after an update).
///
/// A file lands here when opening it took a scan down — crashed it, or hung it forever. Both are
/// real: the desktop app's scan journal converts a died-mid-file scan into an entry here, and
/// `Bionic Delay v1_2.64.dll` earned the first entry by hanging inside `LoadLibrary` without
/// return (measured 2026-08-10, in isolation, 20 s and counting). Format-neutral on purpose —
/// nothing says a VST3 cannot hang too.
pub fn blacklist_path() -> Option<PathBuf> {
    Some(volt_data_dir()?.join("vst-blacklist.txt"))
}

/// The blacklist as it stands, empty if there is none.
pub fn read_blacklist() -> Vec<String> {
    blacklist_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Whether this file is on the blacklist. Compared as text, because that is what the file holds
/// and what a human editing it will have typed.
pub fn is_blacklisted(path: &Path) -> bool {
    let p = path.to_string_lossy();
    read_blacklist().iter().any(|b| b.as_str() == p)
}

/// Add a plugin to the blacklist, once. Returns whether it was newly added.
///
/// Called when a plugin has demonstrably taken the process down — during a scan, or while being
/// loaded into a song. The alternative to remembering is an app that dies the same way every time
/// it is started, which is what `Bionic Delay` did before any of this existed.
pub fn blacklist(path: &Path) -> bool {
    let p = path.to_string_lossy().to_string();
    let mut list = read_blacklist();
    if list.iter().any(|b| *b == p) {
        return false;
    }
    list.push(p);
    if let Some(bp) = blacklist_path() {
        if let Some(dir) = bp.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&bp, list.join("\n"));
    }
    true
}

// ---- ending the process ------------------------------------------------------------------------

/// End the process now, without letting Windows unload the plugin DLLs.
///
/// **This is how a plugin host exits, and it is not a hack.** Once the plugins have been released
/// there is nothing left worth doing tidily, and what remains — Windows walking every loaded DLL's
/// `DllMain(PROCESS_DETACH)` — is code we did not write, running after we no longer care, with a
/// measured habit of falling over. On this machine: Jup-8000 V never returned from it at all (the
/// program hung after printing its results, and had to be killed), and Vital, Diva and Kontakt came
/// back with an access violation. Every one of them had already played correctly and released
/// cleanly. `std::process::exit` does not help: it runs the same teardown.
///
/// So: flush what we have written, then stop. Anything that must be durable has to be on disk
/// before this is called, which is a real constraint and the reason this is one obvious function
/// with a name that says what it does rather than something buried in a `Drop`.
///
/// Off Windows there are no plugin DLLs to fear (the stub host never loads one), so this is a
/// plain `std::process::exit` — same name, same "flush first, nothing after it happens" contract,
/// so callers do not fork on platform.
pub fn hard_exit(code: u32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    #[cfg(windows)]
    {
        // kernel32, linked into every Windows process already — declaring the two calls here is
        // smaller and clearer than taking on windows-sys for them.
        extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn TerminateProcess(handle: *mut std::ffi::c_void, exit_code: u32) -> i32;
        }
        unsafe {
            TerminateProcess(GetCurrentProcess(), code);
        }
        // TerminateProcess on the current process does not return, but the compiler does not know.
        unreachable!("TerminateProcess returned");
    }
    #[cfg(not(windows))]
    std::process::exit(code as i32);
}

// ---- the touch log -----------------------------------------------------------------------------
//
// Plugins TELL their host when a knob in their editor is moved — VST3 through
// `IComponentHandler::performEdit`, VST2 through the `audioMasterAutomate` callback. Every DAW's
// "automate the knob you just touched" is built on exactly this, and VOLT simply never listened.
// One host-wide slot holding the latest touch is all the feature needs: the user turns a knob,
// then asks VOLT to automate "that one". Portable data, portable code — it lives here so
// `last_touch` answers (with nothing) on every platform rather than being another stub.

/// Where a touch came from: each plugin instance gets a tag at instantiate, and the desktop side
/// keeps a tag → slot/track registry. A tag rather than a name because two Sytrus instances are
/// two different targets with one name.
#[cfg_attr(not(windows), allow(dead_code))] // only real plugins are ever tagged
static NEXT_TAG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn next_tag() -> u64 {
    NEXT_TAG.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

struct Touch {
    tag: u64,
    id: u32,
    value: f64,
    at: std::time::Instant,
}

static LAST_TOUCH: Mutex<Option<Touch>> = Mutex::new(None);

#[cfg_attr(not(windows), allow(dead_code))] // nothing off-Windows ever records one
pub(crate) fn record_touch(tag: u64, id: u32, value: f64) {
    if tag == 0 {
        return; // a scan-path instance nobody can address
    }
    *LAST_TOUCH.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Touch { tag, id, value, at: std::time::Instant::now() });
}

/// The most recent plugin-knob touch: (instance tag, param id, normalised value, ms ago).
pub fn last_touch() -> Option<(u64, u32, f64, u64)> {
    LAST_TOUCH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|t| (t.tag, t.id, t.value, t.at.elapsed().as_millis() as u64))
}

#[cfg(test)]
mod scan_skip_tests {
    use super::*;

    /// The exclusion covers both FL wrappers and nothing else. It is matched on the FILE NAME, so
    /// this is checkable without touching the disk — and it needs checking, because the cost of
    /// getting it wrong in either direction is high: too loose and a plugin vanishes with no
    /// explanation, too tight and a scan boots a DAW inside VOLT.
    #[test]
    fn the_scan_skips_fl_studio_vsti_and_only_that() {
        let skip = |s: &str| skip_in_scan(Path::new(s));
        assert!(skip(r"C:\Program Files\Common Files\VST2\FL Studio VSTi.dll"));
        assert!(skip(r"C:\Program Files\Common Files\VST2\FL Studio VSTi (Multi).dll"));
        // case is not a defence: Windows file names are case-insensitive
        assert!(skip(r"C:\vst\fl studio vsti.dll"));
        // …and every other Image-Line plugin is still perfectly welcome
        assert!(!skip(r"C:\Program Files\Common Files\VST2\IL Sytrus.dll"));
        assert!(!skip(r"C:\Program Files\Common Files\VST2\Sytrus VSTi.dll"));
        assert!(!skip(r"C:\Program Files\Common Files\VST2\IL Delay.dll"));
    }
}
