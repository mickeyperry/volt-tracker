# VOLT native

## `desktop/` — what the shell does that a tab cannot

Still just a shell: `beta2.html` runs inside it unchanged, same file the web build serves. What it
adds is three Rust commands, and both of the features they back follow the same shape — **the
feature already existed in the web build, and the desktop supplies a better transport for it.**

```
cargo test --release --manifest-path desktop/Cargo.toml   # 7 tests, hits the real Everything
cargo tauri build --no-bundle                             # from inside desktop/
```

### Sample Search, with nothing to set up

The Sample Search panel and all its logic — Everything's query syntax, ranking, preview,
drag-to-grid — are shared with the web build. Every request it makes goes through one function,
`esGet(path)`, which in a browser has to negotiate obstacles that exist only because a web page is
talking to 127.0.0.1: a CORS header configured on Everything's side, Chrome's Local Network Access
prompt, or the companion bridge extension.

`ev_get` is a third transport for that same function. Same path string, same bytes back, and none
of the negotiation — so in the desktop app the panel just works, with no configuration at all.

It goes over Everything's **HTTP server**, which was not the first choice. Everything here runs
`run_as_admin=1`, and UIPI blocks window messages from a normal-integrity process to an elevated
one, so the classic `Everything_QueryW` SDK route is dead: `FindWindowW` finds none of the IPC
window classes (measured 2026-08-09). The named pipe connects but its wire format is undocumented.
The HTTP server is supported, returns JSON, *and* serves the sample's bytes with the right MIME
type — search and audition over one connection. `src/http.rs` is forty lines of `TcpStream` rather
than an HTTP crate, because the only server VOLT ever talks to is that one, on loopback, in clear.

### Songs as real files

`store.rs` writes songs to a folder — `Documents\VOLT` by default, changeable from the command
palette ("Song folder: change…"). Same JSON the browser's Export writes, and `.json` files are
listed alongside `.volt`, so a song moves between the two builds by sitting in the folder.

The web build's Vault does this with `showDirectoryPicker`, which gives a *handle* whose permission
lapses, and which Firefox does not implement at all. Rust just writes to a path. Autosave mirrors
there on the same timer, and writes through a temp file plus an atomic rename, so a crash mid-save
leaves the previous song intact rather than a truncated one.

**Not yet done:** loading from that folder still goes through the Projects panel and localStorage —
the folder is a mirror, not yet the source of truth. Sample *files* referenced by a song are still
OPFS-side; only the song JSON is on disk.


Three Rust pieces. None of them replaces anything — the web build at `beta2.html` is untouched and
still the same file the shell loads.

## `engine/` — the audio engine, ported not rewritten

The plan in ROADMAP.md was to write 14 instruments and 15 effects as Rust DSP from scratch: ten to
sixteen weeks, and every unit an approximation you could only argue about by ear. That is not what
this is.

VOLT's instruments are not hand-written DSP in the first place. They are Web Audio node graphs —
oscillators, biquads, waveshapers, delays, a convolver, a compressor — wired together in `trig()`.
The [`web-audio-api`](https://crates.io/crates/web-audio-api) crate is that same API, spec
compliant, in Rust. So each instrument ports as a **transliteration** of JS that already exists,
kept deliberately line-for-line so the two can be read side by side, and ASIO comes free
underneath because the crate's backend is the same cpal the probe measured.

```
cargo build --release --manifest-path engine/Cargo.toml
cargo run --release --bin live --manifest-path engine/Cargo.toml          # list output devices
cargo run --release --bin live --manifest-path engine/Cargo.toml -- nio   # play two bars through one
```

### ASIO: the feature flag is not enough

**`cpal-asio` does not give you ASIO.** It compiles the ASIO backend into cpal, but the crate's
`get_host()` (`src/io/cpal.rs`) special-cases PipeWire and JACK and then falls through to
`cpal::default_host()` — which on Windows is always WASAPI. Nothing in the crate ever asks for the
ASIO host, and `enumerate_devices_sync` therefore only ever lists WASAPI devices. Measured here:
`live` lists four devices, all of them WASAPI, while `probe` in the same session reports
`audio APIs available: [Asio, Wasapi]`.

> **Superseded 2026-08-10 — this is now done properly.** `native/vendor/web-audio-api` is a fork
> of the crate that enumerates *every* host instead of one. `live` lists all eight devices with
> their API in the label and opens whichever is asked for, on the right host:
>
> ```
>   [0]  ASIO4ALL v2 [ASIO]                     5.80 ms
>   [2]  Novation USB ASIO Driver [ASIO]       17.33 ms
>   [4]  Line Out (nio 2|4) via USB [WASAPI]   12.00 ms
> ```
>
> The section below is the investigation that got there; the numbers and the two traps at the end
> still stand. Build with `--features asio`.

The ten-line fix works. Adding an ASIO arm to `get_host()` alongside JACK, in a copy of the crate,
makes `enumerate_devices_sync` list `ASIO4ALL v2`, `FL Studio ASIO`, `Novation USB ASIO Driver` and
`UMC ASIO Driver` and opens them. So the question is settled — but the answer is not the one the
commit message claimed.

### What ASIO is actually worth here — 5.8 ms, not 1.45

**The 1.45 ms figure was not a latency.** `probe` prints *callback period* — the block size cpal
was granted, 64 frames ÷ 44100. Output latency is a different quantity: how far ahead of the
speaker the audio we render is, which is what the browser's `baseLatency + outputLatency` reports
and what the 58 ms is. Comparing the two was apples to oranges, and the "forty times better" in
ROADMAP.md came out of that mistake.

Measured properly — every figure below is `output_latency` read from the running stream, same
metric as the browser's, on this machine:

| path | output latency | |
|---|---|---|
| Web Audio in the browser | 58 ms | |
| UMC ASIO Driver | 40.3 ms | 1778 frames @ 44.1k |
| Novation USB ASIO Driver (the nio) | 17.3 ms | 832 frames @ 48k |
| **this engine as shipped, nio via WASAPI** | **12.0 ms** | 576 frames @ 48k |
| **ASIO4ALL v2, patched crate** | **5.8 ms** | 256 frames @ 44.1k |

So the real win is 58 → 5.8 ms, **ten times**, and only through ASIO4ALL. The nio's own ASIO
driver is *worse than WASAPI* on this box, and the Behringer's is worse than the browser.

**ASIO drivers ignore the buffer size we ask for.** Interactive (128 frames), Balanced (512),
Playback (1024) and `Custom(0.00145)` (64) all produce the identical figure on all three drivers —
each one uses whatever is set in its own control panel and reports that back. Nothing in the
engine's code can lower these numbers; the buffer setting in the driver's panel is the only knob,
and it belongs to the user. That is normal DAW behaviour, but it means "VOLT's latency" is not a
number VOLT gets to choose.

Two things about the patch that stop it being a ten-line commit:

- **It makes ASIO the *only* host.** Following the JACK precedent, the moment an ASIO host exists
  it wins and WASAPI devices vanish from the list entirely. A tracker needs both, with the user
  picking — so the honest upstream change is host enumeration, not another `#[cfg]` arm.
- **ASIO4ALL leaves the process undead.** After the context drops, the exe stops responding but
  never exits and holds a lock on its own file; `taskkill` reports "no running instance" while the
  handle is still there. Not fatal for a probe, unacceptable for an app the user closes. The nio
  and UMC drivers both exit cleanly, so it is ASIO4ALL specifically.
  **Measured again 2026-08-10, with two things worth knowing.** They *accumulate* — five runs left
  five stuck processes, each holding ASIO4ALL, which is enough that the driver cannot be opened
  again and the next build cannot relink `live.exe`. And `taskkill /F` does not clear them, but
  PowerShell `(Get-Process live).Kill()` does, within a second or two. `volt_play` now kills the
  whole process tree rather than just the shell it spawned, which is what let them pile up. The
  same teardown runs in the desktop app when a second device is opened — worth checking by
  switching device twice before trusting a long session on ASIO4ALL.

Verdict: ASIO is worth having and worth doing properly later, behind a device picker. It is not
worth blocking the port on, because 12 ms of WASAPI is already good enough to write music to and
the remaining 6 ms costs a forked dependency.

**Ported so far: kick, snare, hat, clap, sub.** Nine instruments and all the effects to go.

**Ported so far: kick, snare, hat, clap, sub.** Nine instruments and all the effects to go.

### Proving it, rather than claiming it

`tests/parity.test.js` renders the same note through both engines into the same bare graph — voice
straight to the destination, no channel strip, no master chain — and subtracts one from the other.
The residue is reported in dB. Where the spec pins the behaviour down, the two agree to between
-85 dB and bit-identical:

| primitive | residue |
|---|---|
| gain envelope, 4 ms decay | **bit-identical** |
| buffer source started off the sample grid | -119 dB |
| biquad (bandpass, highpass) | -102 dB |
| exponential and linear gain ramps | -90 dB |
| waveshaper, 2x oversampled | -86 dB |
| seeded noise buffer | -88 dB |

Two traps cost real time and are worth knowing before porting anything else:

- **`s >> 17` in JavaScript is an arithmetic shift.** `seedRnd` looks like a textbook xorshift32
  but `>>` coerces to int32 first, so once the high bit is set it sign-extends. Get it wrong and
  you still get perfectly good white noise — just not *the same* white noise, which no ear will
  ever catch and a null test catches instantly.
- **Divide in f64 and narrow afterwards.** The JS computes `s / 2^32` at double precision and only
  loses bits when the result lands in a `Float32Array`.

### Where the two engines genuinely differ

Sample-exact parity with Chrome turns out to be unachievable, and the reason is Chrome, not the
port. Two of its AudioParam behaviours don't follow the spec:

- **Frequency automation is read from the start of the render quantum, not the start of the note.**
  A note beginning 29 samples into a quantum runs at the *previous* frequency for those 29 samples
  and then catches up. It lasts 0.7 ms, nobody could hear it, and it shifts the phase of everything
  after it — so a strict null against a mid-quantum note can never pass however correct the port is.
- **A `setTargetAtTime` decay starts one block early.** For the quantum leading up to the event
  Chrome multiplies the parameter by `exp(-(T0 - blockStart) / tc)` where the spec says hold.
  Measured as a flat 1.154% dip, predicted to five digits by `exp(-128 / (0.25 x 44100))`.

Firefox has a different set again, so there is no single "the browser" to null against. The suite
therefore grades on RMS of the residue and prints peak beside it, and the one *real* remaining
difference is listed as a known gap with the number it currently measures:

- **Waveform shape.** The crate's built-in `sawtooth` and `square` are naive — they measured
  -15 dB against Chrome and ran **1.18x hot**, because Chrome normalises *after* band-limiting and
  the Gibbs overshoot eats the headroom. Both are now built from the spec's Fourier series as
  band-limited `PeriodicWave` tables, which fixes the level and the shape together:

  | | before | after |
  |---|---|---|
  | saw @110 Hz | -15.0 dB | **-35.7 dB** |
  | square @110 Hz | -14.9 dB | **-38.3 dB** |
  | square @1760 Hz | -15.1 dB | **-43.9 dB** |
  | saw @1760 Hz | -15.2 dB | -21.3 dB |

  Three things this cost, all of them counter-intuitive and all of them settled by measurement
  rather than argument:

  - **The triangle is left on the built-in.** A table makes it *worse* at both ends of the
    keyboard (-66 → -47 dB at 110 Hz, -38 → -28 at 1760). Its harmonics fall away as 1/k² so it
    barely aliases in the first place, and an 8192-point interpolated table only adds error.
  - **Band-limit per octave, not per note.** Keying tables on the exact Nyquist harmonic count is
    more "correct" and matches Chrome *worse* everywhere — square at 1760 Hz falls from -44 dB to
    -21 dB, because Chrome uses five odd harmonics there where exact Nyquist allows six. Chrome
    buckets by octave; so do we.
  - **`saw @1760 Hz` is the worst case left.** Chrome crossfades between neighbouring octave
    tables and we pick one. It is the top of the spectrum, where browsers disagree anyway.
    Closing it means replicating the crossfade.

  The crate's `PeriodicWave` is a naive 8192-sample table with no band-limiting of its own, so
  handing it the full series would have made aliasing *worse*, not better. The band-limiting here
  is ours: `harmonics()` truncates the series at what fits under Nyquist.

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
