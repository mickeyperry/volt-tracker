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
| **Native ASIO** | **1.45 ms** — asked for 64 frames, granted |

**Forty times better than the browser.** That is the case for a native engine, in one line.

Two things that cost an afternoon, written down because both read as driver faults and neither is:

- Ask for the sample format the driver actually **speaks**. ASIO drivers commonly hand out 32-bit
  *integer*, and asking such a device for f32 is refused with "stream configuration is not
  supported" — which sounds like the buffer size was rejected when it was nothing of the sort.
  Every buffer size looked refused until the format matched; then 64 frames was granted first ask.
- Enumerate every output device, not just each host's default. On Windows the ASIO driver worth
  having is rarely the one Windows considers default.

Registered ASIO drivers here: ASIO4ALL v2, FL Studio ASIO, Novation USB ASIO, UMC ASIO. The nio has
no native driver of its own so it goes through ASIO4ALL — the Behringer UMC202HD does have one and
would be worth measuring separately.

Building with ASIO needs the Steinberg SDK on disk and clang for the bindings:

```
$env:CPAL_ASIO_DIR = "C:\Users\Mickey\sdk\ASIOSDK"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
```

The SDK is **not** in this repo and must not be: its licence forbids redistribution, and this repo
is public. It is dual-licensed (proprietary or GPLv3); shipping a binary built against it under the
proprietary terms needs a signed agreement from Steinberg. Building it for yourself does not.

The app's own side of that comparison comes from `Ctrl+Space → redraw`, which prints
`AudioContext.baseLatency + outputLatency` next to the render figures.
