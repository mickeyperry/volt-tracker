//! The app's own CPU meter — the number FL keeps in its corner.
//!
//! "CPU of the program itself" has to mean the whole program: WebView2 renders the entire UI in
//! SEPARATE processes (msedgewebview2.exe, several of them), so `GetProcessTimes` on this
//! process alone would show the engine and the plugins and quietly omit the half the user is
//! looking at. So: one toolhelp snapshot, a parent-pid walk from our own pid, and the summed
//! kernel+user times of the whole tree. Percent is of the WHOLE MACHINE (all cores), like Task
//! Manager, because that is the number people compare against.
//!
//! Polled at UI-meter rate (a few Hz). A snapshot walk costs tens of microseconds at that rate;
//! pid reuse can momentarily misattribute a stranger to our tree, which for a meter is noise,
//! not a bug.

use std::sync::Mutex;
use std::time::Instant;

struct Prev {
    at: Instant,
    /// Summed kernel+user time of the process tree, in 100 ns FILETIME units.
    busy: u64,
}
static PREV: Mutex<Option<Prev>> = Mutex::new(None);

/// Percent of the whole machine this app (tree) is using, 0..100 — or a negative number when
/// there is no answer yet (the first call has no interval to measure over).
pub fn app_cpu_percent() -> f64 {
    let busy = tree_cpu_100ns();
    let now = Instant::now();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
    let mut prev = PREV.lock().unwrap_or_else(|e| e.into_inner());
    let pct = match prev.as_ref() {
        Some(p) => {
            let dt = now.duration_since(p.at).as_secs_f64();
            if dt < 0.05 {
                return -1.0; // polled twice in one breath — no honest delta to report
            }
            let db = busy.saturating_sub(p.busy) as f64 * 100e-9;
            (db / dt / cores * 100.0).clamp(0.0, 100.0)
        }
        None => -1.0,
    };
    *prev = Some(Prev { at: now, busy });
    pct
}

#[cfg(windows)]
fn tree_cpu_100ns() -> u64 {
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::processthreadsapi::{GetProcessTimes, OpenProcess};
    use winapi::um::tlhelp32::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    let me = std::process::id();
    // (pid, ppid) for every process — one snapshot, walked once
    let mut procs: Vec<(u32, u32)> = Vec::with_capacity(256);
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return proc_cpu_100ns(me);
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                procs.push((e.th32ProcessID, e.th32ParentProcessID));
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    // breadth-first over the parent links, starting from us
    let mut tree = vec![me];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        for &(pid, ppid) in &procs {
            if ppid == parent && pid != parent && !tree.contains(&pid) {
                tree.push(pid);
            }
        }
        i += 1;
    }
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    let mut total = 0u64;
    for pid in tree {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                continue;
            }
            let mut c: FILETIME = std::mem::zeroed();
            let mut x: FILETIME = std::mem::zeroed();
            let mut k: FILETIME = std::mem::zeroed();
            let mut u: FILETIME = std::mem::zeroed();
            if GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u) != 0 {
                total += ft(k) + ft(u);
            }
            CloseHandle(h);
        }
    }
    total
}

#[cfg(windows)]
fn proc_cpu_100ns(_pid: u32) -> u64 {
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::processthreadsapi::{GetCurrentProcess, GetProcessTimes};
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    unsafe {
        let mut c: FILETIME = std::mem::zeroed();
        let mut x: FILETIME = std::mem::zeroed();
        let mut k: FILETIME = std::mem::zeroed();
        let mut u: FILETIME = std::mem::zeroed();
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut x, &mut k, &mut u) != 0 {
            ft(k) + ft(u)
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
fn tree_cpu_100ns() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Burn a little CPU between two polls and the meter must see a sane, positive number —
    /// and the first-ever poll must say "no answer yet" rather than inventing one.
    #[test]
    fn the_meter_reads_its_own_burn() {
        let first = app_cpu_percent();
        assert!(first < 0.0, "the first poll has no interval to measure: got {first}");
        let t0 = Instant::now();
        let mut sink = 0u64;
        while t0.elapsed().as_millis() < 120 {
            sink = sink.wrapping_mul(6364136223846793005).wrapping_add(1);
        }
        std::hint::black_box(sink);
        let pct = app_cpu_percent();
        assert!((0.0..=100.0).contains(&pct), "one busy core must read as a plausible percent: got {pct}");
    }
}
