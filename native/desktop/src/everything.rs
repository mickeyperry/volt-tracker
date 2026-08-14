//! Sample browsing, backed by Everything (voidtools).
//!
//! **This is a transport, not a feature.** The Sample Search panel already exists in beta2.html —
//! search syntax, ranking, preview, drag-to-grid, the lot — and every request it makes goes
//! through one function, `esGet(path)`. In a browser that function has two ways out: a plain
//! `fetch` to loopback, or a relay through the companion extension when the page is hosted. Both
//! exist to get around the same obstacle, which is that a web page is not allowed to talk to
//! 127.0.0.1 without the user arranging permission first.
//!
//! A desktop app has no such problem. So this adds a third transport and nothing else: `ev_get`
//! takes the identical path string and returns the identical bytes. No CORS header to configure,
//! no extension to install, no `targetAddressSpace` prompt — the panel simply works. Every line of
//! searching, ranking and importing above it is shared with the web build.
//!
//! Why HTTP at all, when this is native code that could use the SDK? Because it cannot. Everything
//! on this machine runs `run_as_admin=1`, and UIPI blocks window messages from a normal-integrity
//! process to an elevated one — `FindWindowW` finds none of the IPC window classes, measured
//! 2026-08-09. The named pipe (`allow_ipc_pipe=1`) does connect, but its wire format is
//! undocumented. The HTTP server is a supported Everything feature that returns JSON *and* serves
//! the sample's bytes with the right MIME type, which is the whole job over one connection.

use serde::Serialize;
use std::time::Duration;

use crate::http;

const TIMEOUT: Duration = Duration::from_secs(10);

/// Refuse to hand the webview something absurd. The longest thing anyone samples is a full take;
/// 256 MB is far past that and well short of "the tab dies".
const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Serialize)]
pub struct Status {
    pub ok: bool,
    pub port: u16,
    /// Plain-language next step when `ok` is false. Everything's HTTP server ships disabled, so
    /// "not running" is a setup step rather than a bug, and the message has to say which.
    pub note: String,
}

/// Everything's HTTP server keeps its port in the plugins ini, under the serving DLL's own
/// section. 1.5 ships it as a plugin (`http_server64.dll`); 1.4 has it built in and writes
/// `http_server_port` into the main ini. Read both, newest first, then fall back to the default.
fn configured_port() -> u16 {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return 80;
    };
    let dir = std::path::Path::new(&appdata).join("Everything");

    for (file, key) in [
        ("Plugins-1.5a.ini", "port"),
        ("Plugins.ini", "port"),
        ("Everything-1.5a.ini", "http_server_port"),
        ("Everything.ini", "http_server_port"),
    ] {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        // In the plugins ini the key only means what we want inside the http_server section. The
        // other plugin — everything_server64.dll, the ETP/FTP server — has a `port` of its own,
        // and picking that one up would aim us at a server speaking a different protocol.
        let mut in_http = !file.starts_with("Plugins");
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_http = line.to_ascii_lowercase().contains("http_server");
                continue;
            }
            if !in_http {
                continue;
            }
            if let Some(v) = line.strip_prefix(key).and_then(|r| r.trim().strip_prefix('=')) {
                if let Ok(p) = v.trim().parse::<u16>() {
                    if p != 0 {
                        return p;
                    }
                }
            }
        }
    }
    80
}

/// Where the server is and whether it answers a JSON query. Both matter separately: it can be
/// running happily with `allow_query_access=0`, in which case the port is open and search fails.
///
/// The port comes back so the UI can show it, and so the settings panel can pre-fill itself with
/// the real value instead of a guess.
#[tauri::command(async)]
pub fn ev_status() -> Status {
    let port = configured_port();
    match http::get(port, "/?search=volt&json=1&count=1", TIMEOUT) {
        Ok(r) if r.status == 200 && r.content_type.contains("json") => Status {
            ok: true,
            port,
            note: String::new(),
        },
        Ok(r) if r.status == 200 => Status {
            ok: false,
            port,
            note: format!(
                "Everything is answering on port {port}, but not with JSON. \
                 In Everything: Tools ▸ Options ▸ HTTP Server ▸ allow JSON queries."
            ),
        },
        Ok(r) => Status {
            ok: false,
            port,
            note: format!("Everything's HTTP server returned HTTP {}.", r.status),
        },
        Err(e) => Status {
            ok: false,
            port,
            note: format!(
                "No Everything HTTP server on port {port}. \
                 Open Everything ▸ Tools ▸ Options ▸ HTTP Server and turn it on. ({e})"
            ),
        },
    }
}

/// The transport itself. `path` is whatever `esGet` was going to request — a search query string
/// or a file path — and the reply is the raw body.
///
/// Returned as `tauri::ipc::Response` so a 30 MB WAV crosses to the webview as bytes rather than
/// as a JSON array of numbers, which is the difference between instant and unusable.
///
/// A non-200 becomes an `Err`, because every caller of `esGet` treats a failed request the same
/// way and the status code on its own tells Mickey nothing. The message names the setting to
/// change instead.
#[tauri::command(async)]
pub fn ev_get(path: String) -> Result<tauri::ipc::Response, String> {
    fetch(&path).map(tauri::ipc::Response::new)
}

/// The part with the logic in it. Separate from `ev_get` because `tauri::ipc::Response` is
/// write-only — it can be constructed but never read back — so a test that went through the
/// command could only assert that it did not error, which is barely a test at all.
fn fetch(path: &str) -> Result<Vec<u8>, String> {
    let r = http::get(configured_port(), path, TIMEOUT)?;

    match r.status {
        200 => {}
        403 | 404 if path.len() > 1 && !path.starts_with("/?") => {
            return Err(
                "Everything would not serve that file — its HTTP Server options need \
                 “Allow file download” enabled."
                    .into(),
            )
        }
        401 => return Err("Everything's HTTP server is asking for a username and password.".into()),
        s => return Err(format!("Everything returned HTTP {s}")),
    }

    if r.body.len() > MAX_BYTES {
        return Err(format!(
            "that file is {} MB — too big to load as a sample",
            r.body.len() / 1024 / 1024
        ));
    }
    Ok(r.body)
}

/// These talk to the real Everything on this machine, because that is the only thing worth
/// testing here — `http.rs` is a client for exactly one server, and a mock of that server would
/// only prove the mock matches my assumptions. They skip rather than fail when it is not running,
/// so `cargo test` still passes on a machine without it.
#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Option<u16> {
        let p = configured_port();
        http::get(p, "/?search=volt&json=1&count=1", TIMEOUT)
            .ok()
            .filter(|r| r.status == 200)
            .map(|_| p)
    }

    #[test]
    fn finds_the_port_from_everythings_own_config() {
        // Never the fallback on a machine with Everything installed, and never the ETP server's
        // port (14631) picked up out of the neighbouring ini section.
        let p = configured_port();
        assert_ne!(p, 14631, "picked up the ETP/FTP plugin's port by mistake");
        assert!(p > 0);
    }

    #[test]
    fn searching_returns_json_with_paths() {
        let Some(_) = live() else {
            eprintln!("skipped: Everything's HTTP server is not running");
            return;
        };
        let r = fetch("/?search=ext%3Awav&json=1&path_column=1&count=3").expect("search failed");
        let body = String::from_utf8_lossy(&r);
        assert!(body.contains("totalResults"), "not Everything JSON: {body:.200}");
        assert!(body.contains("\"path\""), "path_column did not come through");
    }

    #[test]
    fn fetches_a_wav_as_bytes() {
        let Some(_) = live() else {
            eprintln!("skipped: Everything's HTTP server is not running");
            return;
        };
        // The parity harness leaves real WAVs on disk; use one of ours rather than hunting the
        // index, so the test does not depend on what Mickey happens to have installed.
        let p = concat!(
            "/C:/Users/Mickey/volt-tracker/tests/.parity/",
            "probe-kickenv-aligned-0-rust.wav"
        );
        match fetch(p) {
            Ok(b) => {
                assert!(b.len() > 1000, "suspiciously small: {} bytes", b.len());
                assert_eq!(&b[..4], b"RIFF", "not a WAV — got {:?}", &b[..4.min(b.len())]);
            }
            // Allowed: the file may not exist on a fresh clone that has never run the harness.
            Err(e) if e.contains("would not serve") => {
                eprintln!("skipped: {e}")
            }
            Err(e) => panic!("unexpected failure: {e}"),
        }
    }

    #[test]
    fn refuses_a_path_with_smuggled_headers() {
        // A search string reaching the request line with a CRLF in it would let a filename append
        // headers of its own. http::get rejects control characters; prove it stays that way.
        assert!(http::get(1, "/?search=a\r\nX-Evil: 1", TIMEOUT).is_err());
        assert!(http::get(1, "no-leading-slash", TIMEOUT).is_err());
    }
}
