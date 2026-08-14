//! Songs as files in a folder, which is what Mickey asked for: *"we need it to access direct
//! mothafkn storage"*.
//!
//! The web build keeps songs in localStorage and OPFS — invisible, per-origin, and impossible to
//! back up with anything but VOLT's own export. In a desktop app that is backwards. Here a song is
//! a `.volt` file in a folder you chose, so Explorer, Dropbox, a USB stick and `git` all work on
//! it without VOLT's permission.
//!
//! The format does not change. A `.volt` file is exactly the JSON that Export already writes, so
//! anything saved here loads in the browser build and anything exported from the browser drops
//! into this folder and appears. That is the whole compatibility story, and it is why this is a
//! storage backend rather than a new file type.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub full: String,
    pub size: u64,
    /// Milliseconds since the epoch, so JS can `new Date(m)` it without conversion.
    pub modified: u64,
}

/// Where songs live unless told otherwise: `Documents\VOLT`. Created on first use.
#[tauri::command(async)]
pub fn lib_default() -> String {
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Documents").join("VOLT").to_string_lossy().into()
}

/// A song filename is user input that ends up in a path, so it is the one place a traversal could
/// get in. Keep the basename and nothing else — no separators, no drive letters, no `..`.
fn safe_name(name: &str) -> Result<String, String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty() || base == "." || base == ".." || base.contains(':') {
        return Err("not a usable filename".into());
    }
    if base.len() > 180 {
        return Err("filename is too long".into());
    }
    Ok(if base.to_ascii_lowercase().ends_with(".volt") {
        base.to_string()
    } else {
        format!("{base}.volt")
    })
}

#[tauri::command(async)]
pub fn lib_list(dir: String) -> Result<Vec<Entry>, String> {
    let dir = PathBuf::from(&dir);
    if !dir.exists() {
        // Not an error — an empty library is the normal first run.
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
        return Ok(Vec::new());
    }

    let rd = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {dir:?}: {e}"))?;
    let mut out: Vec<Entry> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
            // `.json` too, so songs exported from the browser build are visible without renaming.
            if ext != "volt" && ext != "json" {
                return None;
            }
            let md = e.metadata().ok()?;
            if !md.is_file() {
                return None;
            }
            Some(Entry {
                name: p.file_name()?.to_string_lossy().into(),
                full: p.to_string_lossy().into(),
                size: md.len(),
                modified: md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            })
        })
        .collect();

    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

#[tauri::command(async)]
pub fn lib_read(dir: String, name: String) -> Result<String, String> {
    let p = Path::new(&dir).join(safe_name(&name)?);
    std::fs::read_to_string(&p).map_err(|e| format!("cannot read {p:?}: {e}"))
}

/// Write a song, without ever leaving a half-written file where a song used to be.
///
/// Autosave runs on a timer, so the crash window matters: a truncated `.volt` is a lost song, and
/// losing a song is the one unforgivable failure. Write to a sibling temp file, flush it to the
/// platter, and only then rename over the target — rename is atomic, so a reader sees the old
/// song or the new one and never a partial one.
#[tauri::command(async)]
pub fn lib_write(dir: String, name: String, text: String) -> Result<String, String> {
    let dir = PathBuf::from(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;

    let target = dir.join(safe_name(&name)?);
    let tmp = target.with_extension("volt.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("cannot write {tmp:?}: {e}"))?;
        f.write_all(text.as_bytes())
            .map_err(|e| format!("write failed: {e}"))?;
        f.sync_all().map_err(|e| format!("flush failed: {e}"))?;
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not replace {target:?}: {e}")
    })?;
    Ok(target.to_string_lossy().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("volt-store-test-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_song_survives_a_round_trip() {
        let d = tmp("roundtrip");
        let dir = d.to_string_lossy().to_string();
        // The folder does not exist yet — first run must create it rather than fail.
        assert!(lib_list(dir.clone()).unwrap().is_empty());

        lib_write(dir.clone(), "my song".into(), "{\"n\":1}".into()).unwrap();
        let items = lib_list(dir.clone()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "my song.volt", "extension not applied");
        assert_eq!(lib_read(dir.clone(), "my song".into()).unwrap(), "{\"n\":1}");

        // Saving again must replace, not accumulate — autosave writes constantly.
        lib_write(dir.clone(), "my song.volt".into(), "{\"n\":2}".into()).unwrap();
        assert_eq!(lib_list(dir.clone()).unwrap().len(), 1);
        assert_eq!(lib_read(dir.clone(), "my song".into()).unwrap(), "{\"n\":2}");

        // No .tmp left lying about after the atomic rename.
        assert!(!std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "tmp")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn browser_exports_show_up_too() {
        // Export from the web build writes .json. It has to be visible here or the two builds
        // don't actually share a folder, which is the whole point.
        let d = tmp("json");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("from-browser.json"), "{}").unwrap();
        std::fs::write(d.join("notes.txt"), "ignore me").unwrap();
        let items = lib_list(d.to_string_lossy().into()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "from-browser.json");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_song_name_cannot_escape_the_folder() {
        // Song names are typed by the user and land in a path. `..\..\` in a name must not be
        // able to write outside the folder they chose.
        let d = tmp("escape");
        let dir = d.to_string_lossy().to_string();
        for bad in ["../../evil", "..\\..\\evil", "C:\\Windows\\evil", "..", ""] {
            let r = lib_write(dir.clone(), bad.into(), "x".into());
            if let Ok(written) = r {
                assert!(
                    PathBuf::from(&written).starts_with(&d),
                    "{bad:?} escaped to {written}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Read any file the user explicitly pointed at — used by the sample importer for drag-and-drop
/// and the folder picker, where the path comes from the OS rather than from a search result.
#[tauri::command(async)]
pub fn file_read(path: String) -> Result<Vec<u8>, String> {
    let md = std::fs::metadata(&path).map_err(|e| format!("cannot open {path}: {e}"))?;
    if md.len() > 256 * 1024 * 1024 {
        return Err("that file is too big to load".into());
    }
    std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))
}
