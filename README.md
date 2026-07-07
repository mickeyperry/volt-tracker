# VOLT /// — DnB & Trance Tracker

A single-file browser tracker (FastTracker-style) with a built-in synth engine, sampler, and [voidtools Everything](https://www.voidtools.com/) integration for instant sample search across the whole machine.

![tracker](https://img.shields.io/badge/single%20file-volt.html-3fe0b4)

## Run it

**Recommended (full features):** serve `volt.html` through Everything's HTTP server so the page is same-origin with the search API:

1. Everything → Tools → Options → HTTP Server → enable (e.g. port `8667`, optional user/pass).
2. Open `http://localhost:8667/C%3A/path/to/volt.html` in Chrome/Edge.

Opening `volt.html` directly (`file://`) also works, but the browser then blocks the Everything integration (CORS) — pattern editing, synths, and manually-loaded samples still work fine.

**Hosted (GitHub Pages) + Everything search:** the hosted page can use your local Everything server if Everything sends a CORS header for your origin. In Everything 1.5a the HTTP server is a plugin; close Everything, edit `%APPDATA%\Everything\Plugins-1.5a.ini` under `[http_server64.dll]`:

```ini
bindings=127.0.0.1
username=
password=
header=Access-Control-Allow-Origin: https://YOURNAME.github.io
allow_file_download=1
```

then start Everything again. Auth must be off because browsers can't answer a password challenge during a CORS request — binding to `127.0.0.1` keeps the server local-only, and the `header=` origin scope means only your own site can read results.

## Features

- 8-track pattern editor, hex FX column (`Rxx` retrig, `Sxx` sample offset, `U/Dxx` slide, `Gxx` glide, `Cxx` cutoff)
- Synth engine: kick, snare, hat, clap, sub, reese, supersaw, pluck + sampler
- Per-track volume / ping-pong delay / reverb / kick-ducking, mute & solo
- DnB and Trance starter templates, song order list, JSON export/import, localStorage autosave
- **Everything sample search**: type a query in the sidebar, preview (▶) and add (+) results as sampler instruments. ⚙ settings: host/port/user/pass/extension filter
- **Copy to project folder**: added samples are copied into a folder you pick (File System Access API, Chrome/Edge; the handle persists in IndexedDB)
- **Selection & clipboard**: Shift+arrows or mouse drag; `Ctrl+A` cycles column → track → whole pattern; `Ctrl+C/X/V`, `Del` clears

## Keys

`Z–M` / `Q–P` notes · `A` note-off · `Space` play/stop · `[` `]` octave · `Tab` next track · `Shift+arrows` select · `Ctrl+A` column/track/all · `Ctrl+C/X/V` copy/cut/paste · `Ctrl+Z/Y` undo/redo · `Alt+↑/↓` ±semitone, `Alt+Shift+↑/↓` ±octave

## Notes

- Songs autosave per browser origin — use Export/Import to move songs between copies.
- Samples are embedded base64 in the song JSON, so exported songs are fully portable.
