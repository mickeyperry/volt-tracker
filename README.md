# VOLT /// — DnB & Trance Tracker

**▶ Open it: [mickeyperry.github.io/volt-tracker](https://mickeyperry.github.io/volt-tracker/)**

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

**Hosted + browser extension (recommended, no CORS/permission fuss):** the `extension/` folder is a tiny MV3 extension that relays search/download requests from the hosted page to `127.0.0.1:8667`. The page auto-detects it and shows "Everything bridge extension connected".

- Chrome/Edge: `chrome://extensions` → enable Developer mode → **Load unpacked** → pick the `extension/` folder. For a locally-opened `volt.html` (double-click, `file://`) also enable **Allow access to file URLs** on the extension card.
- Firefox: `about:debugging` → This Firefox → **Load Temporary Add-on** → pick `extension/manifest.json` (temporary loads reset on restart; zip + sign for permanent).

Security: it only injects on `mickeyperry.github.io` (fork = edit `matches` + `header=`), only performs GETs, and only to loopback — nothing can leave the machine.

## Features

- 8-track pattern editor, hex FX column (`Rxx` retrig, `Sxx` sample offset, `U/Dxx` slide, `Gxx` glide, `Cxx` cutoff)
- Synth engine: kick, snare, hat, clap, sub, reese, supersaw, pluck, **Psy Bass** (tight self-closing full-on bass), **W0RM-303** acid + sampler
- Per-track volume / ping-pong delay / reverb / kick-ducking / 3-band EQ, mute & solo (hotkeys + dimmed columns), renameable tracks (double-click header or `F2`)
- **Mixer panel** (`F9` or Mixer button): channel strips with fader, live meters, EQ, sends, mute/solo
- Song strip: drag chips to reorder patterns; playing chip fills with loop progress, header shows a pattern-loop progress bar
- Resizable side panel (drag the divider), collapsible sections
- **Render to WAV** (⬇ WAV button): offline-renders the whole song — works on the hosted version too, no server needed
- **Preset browser**: categorized starting points (Drums / Bass / Lead & Pad) instead of raw synth types
- Per-channel **compressor + drive** on top of EQ; master **peak meter** with clip indicator in the top bar
- **Command palette** (`Ctrl+Space`): type to search any action (note-off, transpose, render, mute…) and run it
- Genre starting kits (Kit dropdown): **Full-on (main)** — the baked-in default song — plus DnB 174, Trance 138, Full-on basic 145, Goa 143 — song order list, JSON export/import, localStorage autosave
- **Everything sample search**: type a query in the sidebar, preview (▶), add as new (+, never overwrites) or replace the selected instrument (⇄, asks first). ⚙ settings: host/port/user/pass/extension filter
- **Copy to project folder**: added samples are copied into a folder you pick (File System Access API, Chrome/Edge; the handle persists in IndexedDB)
- **Selection & clipboard**: Shift+arrows or mouse drag; `Ctrl+A` cycles column → track → whole pattern; `Ctrl+C/X/V`, `Del` clears
- **Project tabs**: several songs open side by side; the clipboard works across tabs, so copy in one project and paste in another. `×` closes a tab via a dialog with **Export & close** / Close without saving / Cancel (closing deletes it from the browser). Double-click a tab to rename; Import opens the file in a new tab
- **Themes** (top bar): VOLT Night · Pitch Black (dark room) · Warm Dim (low-blue, evening) · Daylight (bright room)

## Keys

`Z–M` / `Q–P` notes · `A` note-off · `Space` play/stop · `[` `]` octave · `Tab` next track · `Shift+arrows` select · `Ctrl+A` column/track/all · `Ctrl+C/X/V` copy/cut/paste · `Ctrl+Z/Y` undo/redo · `Alt+↑/↓` ±semitone, `Alt+Shift+↑/↓` ±octave · `Alt+1–0` mute track, `Alt+Shift+1–0` solo · `Alt+M`/`Alt+S` mute/solo current track · `Alt+X` all audible · `F2` rename track · `F9` mixer · `Ctrl+Enter` new pattern, `Ctrl+D` duplicate

## Notes

- Songs autosave per browser origin — use Export/Import (or just **drag & drop** a `.json` onto the page — it opens as a new tab; dropped audio files become sampler instruments) to move songs between copies.
- Samples are embedded base64 in the song JSON, so exported songs are fully portable.
