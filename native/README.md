# VOLT native

Two Rust pieces. Neither replaces anything — the web build at `beta2.html` is untouched and still
the same file both of these use.

## `desktop/` — VOLT as a Windows app

A Tauri shell. It loads `beta2.html` and nothing else; there is no bundler, no transform, no
second copy of the app to keep in sync. `tools/mkdesktop.js` copies the one file into
`desktop/dist/index.html` before each build, and that is the whole build step.

What the shell buys, which a browser cannot:

- **A real origin instead of `file://`** — the File System Access folder pickers work, so the
  library and projects folders finally exist on Firefox's home turf. `AudioWorklet` can also load
  a module, which it flatly refuses to do from `file://`.
- **WebView2, i.e. Chromium** — measured about three times faster than Firefox at the grid's DOM
  work, which is where VOLT spends its frames.
- A window with a name and an icon rather than a tab.

What it does **not** buy: audio latency. The webview uses the same Web Audio path as Chrome, so
that number is unchanged. See `probe/` for what a native engine would actually be worth.

```
cargo tauri dev      # run it, with the file live
cargo tauri build    # produces target/release/bundle/nsis/VOLT_<v>_x64-setup.exe
```

Needs Rust and the Tauri CLI (`cargo install tauri-cli --version "^2"`). WebView2 ships with
Windows 10/11. The installer is unsigned, so SmartScreen will warn until it is — that is a
certificate, not an engineering problem.

## `probe/` — the latency question, answered with a number

Throwaway. It enumerates the audio APIs, asks each device for progressively smaller buffers and
reports what it is actually handed, so "should VOLT get a native audio engine" can be argued from
measurements rather than folklore.

```
cargo run --release --manifest-path probe/Cargo.toml
```

Measured on Mickey's machine (nio 2|4, 48 kHz):

| path | latency |
|---|---|
| Web Audio, as VOLT runs today | **58 ms** (10 ms buffer + 48 ms device) |
| Native WASAPI | **10 ms** — a fixed 480-frame callback; requests for 64/128/256 are ignored |

Nearly six times better before ASIO is even in the picture. ASIO is where single digits would come
from and needs Steinberg's SDK plus clang to build against — that is the next probe, and it decides
whether the floor is 10 ms or 3.

The app's own side of that comparison comes from `Ctrl+Space → redraw`, which prints
`AudioContext.baseLatency + outputLatency` next to the render figures.
