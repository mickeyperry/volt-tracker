//! The non-Windows facade — the same front door, with honest answers behind it.
//!
//! Plugin hosting is Windows through and through: `LoadLibrary`, COM vtables, Win32 windows, the
//! `%CommonProgramFiles%\VST3` folder. None of that exists on Linux or macOS, and pretending it
//! does would produce the worst kind of bug — a plugin that "loads" and plays silence with no
//! cause. So this module answers every question truthfully instead: scans find nothing (and count
//! nothing as skipped), `Module::open` fails with a sentence the status bar can show verbatim,
//! editor pumps pump zero messages, and the touch log (which lives in `lib.rs`, shared) simply
//! never has anything to report.
//!
//! **The surface is `win.rs`'s, kept identical by hand** — same names, same signatures — so
//! `volt-engine`'s `vst.rs` and every `vst_*`/`fxvst_*` command in the desktop crate compile
//! unchanged. The page already surfaces these commands' error strings in the status bar, which is
//! how a Linux user learns the score: the whole engine and all fourteen ported instruments over
//! ALSA/CoreAudio, no VST hosting yet.
//!
//! Two deliberate asymmetries, both documented on the Windows side too:
//!   * `Plugin` can never be constructed here (`Module::open` and `instantiate` always fail), so
//!     every method on it is unreachable in practice — they still answer sanely, because a stub
//!     that panics is a stub that takes the app down the day someone finds a new path to it.
//!   * `win.rs` re-exports `vst3::…::Event` and its COM helper types (`Host`, `Stream`,
//!     `HostParams`); those are implementation details no upstream crate names, and they have no
//!     twin here on purpose — adding one would mean shipping fake COM.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{ParamDesc, PluginFormat, PluginInfo};

/// The one sentence every refusal carries. The desktop's `vst_load`/`fxvst_load` pass it to the
/// page, and the page shows command errors in the status bar — so this string IS the UX.
const NOT_AVAILABLE: &str = "VST hosting is not available on this platform yet";

/// Where VST3 plugins conventionally live on this platform — kept so the constant exists for
/// callers (`vstprobe`, the desktop scan) even though [`scan`] deliberately finds nothing yet.
#[cfg(target_os = "macos")]
pub const VST3_DIR: &str = "/Library/Audio/Plug-Ins/VST3";
#[cfg(not(target_os = "macos"))]
pub const VST3_DIR: &str = "/usr/lib/vst3";

/// VST2 folders. There is no convention worth encoding for a host that cannot load one.
pub const VST2_DIRS: &[&str] = &[];

/// What a VST2 scan found: nothing, and — honestly — nothing skipped either. `skipped_32bit`
/// stays 0 rather than some sentinel, so the UI's "N skipped (32-bit)" line simply never shows.
pub struct Vst2Scan {
    pub dlls: Vec<PathBuf>,
    pub skipped_32bit: usize,
}

/// The VST2 scan, which finds nothing here. See the module doc.
pub fn scan_vst2() -> Vst2Scan {
    Vst2Scan { dlls: Vec::new(), skipped_32bit: 0 }
}

/// The VST3 scan, which finds nothing here — deliberately not a directory walk of [`VST3_DIR`],
/// because listing plugins that `Module::open` must then refuse one by one would journal and log
/// a failure per file for a fact one line covers.
pub fn scan(_dir: &Path) -> Vec<PathBuf> {
    Vec::new()
}

/// A plugin file. Cannot actually be opened on this platform; the type exists so upstream
/// signatures (`Arc<Module>`, `module.classes()`, `module.instantiate(…)`) compile unchanged.
pub struct Module {
    _no: (),
}

impl Module {
    /// Always the honest refusal — this is the string the status bar shows when a song that
    /// carries plugins is opened on Linux or macOS.
    pub fn open(_path: &Path) -> Result<Arc<Module>, String> {
        Err(NOT_AVAILABLE.into())
    }

    pub fn classes(&self) -> Vec<PluginInfo> {
        Vec::new()
    }

    pub fn instantiate(
        self: &Arc<Self>,
        _info: &PluginInfo,
        _sample_rate: f64,
        _max_block: i32,
    ) -> Result<Plugin, String> {
        Err(NOT_AVAILABLE.into())
    }
}

/// A live plugin instance — unconstructible here (both doors above are closed), but fully shaped,
/// so `PluginBox`, the retire channel and every field access upstream compile as they do on
/// Windows. Methods answer sanely anyway; see the module doc for why they do not panic.
pub struct Plugin {
    pub name: String,
    /// Channels on the main output bus — what the graph node is built with.
    pub out_channels: usize,
    pub in_channels: usize,
    /// Zero means it cannot be played. Nothing here can be played.
    pub event_in_buses: usize,
    pub parameters: usize,
    /// This instance's address in the touch log — `last_touch()` reports it back.
    pub tag: u64,
    /// The automatable parameters, id + name. What `set_param` would take.
    pub param_list: Vec<ParamDesc>,
    /// What the plugin says it delays by.
    pub latency: u32,
    pub format: PluginFormat,
    _no: (),
}

impl Plugin {
    pub fn process(
        &mut self,
        events: &HostEvents,
        out: &mut [Vec<f32>],
        frames: usize,
    ) -> Result<(), String> {
        self.process_io(events, &[], out, frames)
    }

    /// Silence of the right shape, then the refusal — the same "never hand back garbage buffers"
    /// contract the real `process_io` keeps on its own error paths.
    pub fn process_io(
        &mut self,
        _events: &HostEvents,
        _input: &[&[f32]],
        out: &mut [Vec<f32>],
        frames: usize,
    ) -> Result<(), String> {
        for ch in out.iter_mut() {
            ch.resize(frames, 0.0);
            ch.fill(0.0);
        }
        Err(NOT_AVAILABLE.into())
    }

    pub fn set_param(&mut self, _id: u32, _value: f64) {}

    /// The song's tempo. Kept in the same shape as the real one (nothing to tell) so the engine's
    /// per-block transport broadcast compiles and runs unchanged.
    pub fn set_tempo(&mut self, _bpm: f64) {}

    /// The song's musical position and whether the transport is rolling. Same story as
    /// `set_tempo`: the engine sends it every block; there is no plugin here to hear it.
    pub fn set_transport(&mut self, _playing: bool, _ppq: f64) {}

    pub fn flush_state(&mut self) {}

    /// No patch: a plugin that does not exist has nothing to remember.
    pub fn state(&self) -> Option<Vec<u8>> {
        None
    }

    pub fn set_state(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Err(NOT_AVAILABLE.into())
    }

    pub fn take_editor(&mut self) -> Option<EditorHandle> {
        None
    }
}

// ---- the event list ------------------------------------------------------------------------
//
// Real, not stubbed: it is plain data (the COM wrapper is a Windows detail), the engine's
// `VstProcessor` builds one unconditionally, and keeping the note→MIDI conversion identical here
// means a future non-Windows host inherits behaviour that already matches the tests on the
// Windows side (`events.rs`: +12, velocity clamped).

/// One note out of the list, with the COM and the unions stripped away.
#[derive(Debug, Clone, Copy)]
pub struct PlainNote {
    pub on: bool,
    /// MIDI pitch, 0–127 once clamped by the consumer.
    pub pitch: i16,
    /// 0..1, as VST3 has it.
    pub velocity: f32,
    /// Samples into the block.
    pub offset: i32,
}

/// The host's note list for one block: fill it, hand it to `process`, clear it, repeat.
/// Same single-thread discipline as the Windows original (`RefCell`, no lock).
pub struct HostEvents {
    notes: RefCell<Vec<PlainNote>>,
}

impl Default for HostEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl HostEvents {
    pub fn new() -> HostEvents {
        HostEvents { notes: RefCell::new(Vec::with_capacity(64)) }
    }

    pub fn clear(&self) {
        self.notes.borrow_mut().clear();
    }

    pub fn len(&self) -> usize {
        self.notes.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A note on, `offset` samples into the coming block. VOLT note numbers are not MIDI's —
    /// the +12 is the same conversion `events.rs` documents and tests on Windows.
    pub fn note_on(&self, note: f32, velocity: f32, offset: i32) {
        self.notes.borrow_mut().push(PlainNote {
            on: true,
            pitch: (note.round() as i16) + 12,
            velocity: velocity.clamp(0.0, 1.0),
            offset,
        });
    }

    pub fn note_off(&self, note: f32, offset: i32) {
        self.notes.borrow_mut().push(PlainNote {
            on: false,
            pitch: (note.round() as i16) + 12,
            velocity: 0.0,
            offset,
        });
    }

    /// Read the list back, note by note — the translation hook the VST2 side uses on Windows.
    pub fn for_each_note(&self, mut f: impl FnMut(PlainNote)) {
        for n in self.notes.borrow().iter() {
            f(*n);
        }
    }
}

// ---- the editor ------------------------------------------------------------------------------

pub mod editor {
    //! Plugin windows, on a platform with no plugins: handles that never open, a pump with no
    //! queue. The desktop's plugin thread (`plugins.rs`) runs against this unchanged — it spins
    //! harmlessly, pumping nothing, which is exactly what "do not fork the command surface" costs.

    /// Everything needed to open a plugin's window later. Nothing here can produce one
    /// (`Plugin::take_editor` is always `None`), so this only exists to be named in signatures.
    pub struct EditorHandle {
        /// For the window's title bar.
        pub name: String,
    }

    /// An open plugin window — unreachable here, since [`Editor::open`] always refuses.
    pub struct Editor {
        name: String,
    }

    impl Editor {
        pub fn open(handle: EditorHandle) -> Result<Editor, String> {
            Err(format!("{}: {}", handle.name, super::NOT_AVAILABLE))
        }

        pub fn size(&self) -> (i32, i32) {
            (0, 0)
        }

        pub fn name(&self) -> &str {
            &self.name
        }

        pub fn show(&self) {}

        pub fn hide(&self) {}

        /// This window's handle as a plain number, for matching against the request queues below.
        /// Zero because there is no window; nothing can match it, which is the correct outcome
        /// when the queues are always empty.
        pub fn window_id(&self) -> usize {
            0
        }

        pub fn set_title(&self, _title: &str) {}
    }

    /// One pass of the message queue that does not exist. Zero messages, honestly counted.
    pub fn pump_messages() -> u32 {
        0
    }

    // The three request queues the Windows pump fills when a keystroke is typed INTO a plugin
    // window — Ctrl+Delete to remove the plugin, Alt+[ / Alt+] to walk the channel's chain, Space
    // for the transport. With no plugin windows there is nothing to type into, so each answers
    // empty forever and the plugin thread's drain loops simply never do any work.

    /// Plugin windows that asked to be removed since the last call.
    pub fn take_delete_requests() -> Vec<usize> {
        Vec::new()
    }

    /// Plugin windows that asked to hand over to the next (or previous) effect, `(id, direction)`.
    pub fn take_cycle_requests() -> Vec<(usize, i32)> {
        Vec::new()
    }

    /// How many transport toggles the plugin windows have asked for since last asked.
    pub fn take_space_requests() -> u32 {
        0
    }

    /// Thread setup for editors that will never open (OLE is a Windows concept). A no-op so the
    /// plugin thread's startup sequence is identical on every platform.
    pub fn init_thread() {}
}

pub use editor::{Editor, EditorHandle};
