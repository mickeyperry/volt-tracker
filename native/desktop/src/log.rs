//! A log file, because `eprintln!` in a windowed app goes nowhere.
//!
//! VOLT's native side has spent this whole project debugging blind: the audio thread, the plugin
//! thread and every Tauri command print to a console that a GUI process does not have, so when
//! something fails on Mickey's machine the only evidence is whatever reached the status bar. That
//! turned "the VST window will not open" into an afternoon of inference.
//!
//! `<data dir>/volt.log` (`%LOCALAPPDATA%\VOLT\volt.log` on Windows — see [`volt_dir`]), appended,
//! one line per event, with a timestamp and the thread that wrote it. Truncated at startup past a
//! megabyte so it cannot grow without bound.
//!
//! Deliberately not a logging framework: this is thirty lines, has no dependencies, and never
//! blocks the audio thread for long enough to matter (a line is a few hundred bytes to the OS
//! cache). If it ever does, the answer is to stop logging from the audio thread — not to add a
//! crate.

use std::io::Write;
use std::sync::Mutex;

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// The one VOLT data folder, for EVERYTHING this crate writes: the log, the three journals, the
/// scan cache, the tap. It is `volt_vsthost::volt_data_dir` re-exposed rather than a second
/// implementation, because the blacklist that crate keeps lives in the same folder and two
/// answers to "where is the VOLT folder" would eventually disagree. Per platform:
/// `%LOCALAPPDATA%\VOLT`, `$XDG_DATA_HOME/VOLT` (or `~/.local/share/VOLT`),
/// `~/Library/Application Support/VOLT`.
pub fn volt_dir() -> Option<std::path::PathBuf> {
    volt_vsthost::volt_data_dir()
}

fn path() -> Option<std::path::PathBuf> {
    volt_dir().map(|d| d.join("volt.log"))
}

/// Open the log, rolling it if it has got large. Called once at startup.
pub fn init() {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A megabyte is a few thousand lines — plenty of history, and nothing anyone has to manage.
    if std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }
    write(&format!("---- VOLT started, build {} ----", env!("CARGO_PKG_VERSION")));
}

/// One line. Cheap, non-blocking in practice, and never panics — a logger that can take the app
/// down is worse than no logger.
pub fn write(msg: &str) {
    // **Local time, not UTC.** These lines are read next to a clock and next to what somebody
    // remembers doing — "it froze around ten past" — and a three-hour offset makes that comparison
    // an arithmetic exercise every time. `SystemTime` only knows UTC and Rust's std has no local
    // time at all, so this asks Windows, which is two lines and no dependency.
    let (h, m, s) = local_hms();
    let thread = std::thread::current().name().unwrap_or("?").to_string();
    let line = format!("{h:02}:{m:02}:{s:02} [{thread}] {msg}\n");
    // Mirrored to stderr so `cargo run` and the CLI tools still show it.
    eprint!("{line}");
    if let Ok(mut g) = FILE.lock() {
        if let Some(f) = g.as_mut() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}

/// The wall clock, as the person reading this log sees it.
#[cfg(windows)]
fn local_hms() -> (u16, u16, u16) {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    extern "system" {
        fn GetLocalTime(t: *mut SystemTime);
    }
    let mut t = SystemTime::default();
    unsafe { GetLocalTime(&mut t) };
    (t.hour, t.minute, t.second)
}

/// The same question to POSIX: `localtime_r`, declared by hand for the same reason the Windows
/// branch declares `GetLocalTime` — Rust's std has no local time, and a whole `libc` dependency
/// for one call is the crate this file promised never to add. Only the first three `tm` fields
/// are read; the struct is declared in full (glibc and the BSDs agree on this layout) so the
/// callee has the room it expects to write into.
#[cfg(not(windows))]
fn local_hms() -> (u16, u16, u16) {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const std::ffi::c_char,
    }
    extern "C" {
        fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Zeroed, not Default: a raw pointer has no Default, and all-zeroes is a valid Tm.
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    if unsafe { localtime_r(&secs, &mut tm).is_null() } {
        return (0, 0, 0);
    }
    (tm.tm_hour as u16, tm.tm_min as u16, tm.tm_sec as u16)
}

/// `log!("thing {} happened", x)` — same shape as `println!`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
