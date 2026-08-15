# VOLT — working notes for whoever picks this up

VOLT is a tracker / DAW that runs entirely in a browser tab. One HTML file, no build step, no
dependencies at runtime. Mickey uses it to actually make music, so **breaking his songs is the
one unforgivable failure** — everything below is aimed at that.

Live: https://mickeyperry.github.io/volt-tracker/beta2.html (current) ·
[beta.html](https://mickeyperry.github.io/volt-tracker/beta.html) (previous, frozen)

---

## Talk to Mickey like this

**Short.** TL;DR by default — a line or a few bullets, not essays. He's asked more than once.
Lead with the answer; offer detail instead of dumping it. Long comparisons, staged plans and
rationale belong in a file (this one, or `ROADMAP.md`), not in chat.

He is not a professional programmer. Explain in plain language, no jargon dumps. When he says
something is confusing, that's real feedback — simplify, don't re-explain louder.

## The files

| file | what it is |
|---|---|
| `beta2.html` | **where all work happens.** ~700 KB, one big inline `<script>`. |
| `beta.html` | the previous beta, frozen for comparison. Don't edit it — the newer suites test features it doesn't have, so `VOLT_FILE=beta.html` will fail them, and that's expected. |
| `volt.html` | the stable build. Don't touch unless promoting a release. |
| `tests/` | `node tests/run.js` — 675 checks against `beta2.html`, must stay green. |
| `native/engine/` | the Rust audio engine — a **transliteration** of `trig()`, not a rewrite. See `native/README.md`. `src/mixer.rs` is `ensureChans()`/`applyMix()` **and** `buildGraph()`'s master bus; `src/vst.rs` makes a plugin a node in the graph. |
| `native/vsthost/` | the VST3 host: scan, load, COM, note events. `vstprobe` lists every plugin and plays one. Separate crate on purpose — a plugin is somebody else's code in our process. |
| `tests/profile.js` | `node tests/profile.js` — not a test. Prints where the time actually goes (switch, undo, autosave, frame gaps) so a speed claim can be argued from numbers. |
| `tools/opfs-probe.html` | standalone storage probe (open in a browser, reports what works). |
| `tools/volt-mcp.js` | **the MCP server — use it.** Registered as `volt`, dependency-free stdio JSON-RPC. |
| `native/vendor/web-audio-api` | VOLT's fork of the crate: enumerates every audio host, so ASIO is selectable. Edits marked `VOLT FORK`. |
| `ROADMAP.md` | **the feature list and progress.** Read it when he asks "what's next". Tick items off there. |
| `docs/daw-engineering-bible/` | **how a real DAW engine is built** — 16 docs Mickey commissioned as a reference (WIP, added 2026-08-11). Read the ones its README's reading map names before engine work; `03_DSP_GRAPH` is the one to read before touching the mixer, `07_PLUGIN_ARCHITECTURE` before the plugin host. It describes a **target**, not what exists — see the note below on where it and VOLT actually diverge. |

All three builds share an origin but **never share data**: line 5 of each beta sets
`window.VOLT_NS` (`voltbeta` / `voltbeta2`) and rewrites every `volt.*` localStorage key to
`<ns>.*`; stable has no shim and writes `volt.*` directly. `VNS` in the main script keys OPFS and
IndexedDB off the same thing. If you add persistent storage, namespace it — a bug in a beta must
not be able to reach his real songs. **Verified end to end 2026-08-09**: beta2 saving cannot alter
a song beta saved, in the same browser profile.

The corollary bites when you *enumerate*. `localStorage.key(i)` returns RAW keys — the shim only
wraps get/set — so all three builds' keys sit in one list, and every build's projects look like
`<something.>volt.proj.<id>`. Match your own prefix **exactly** (`projPrefix()`), or you list other
builds' songs as phantom projects that can never be opened, because reading them back goes through
the shim into your own namespace. `ssRefs` is loose *on purpose* by contrast — it protects samples
referenced by any build from the collector, which errs safe. `tests/home.test.js` plants foreign
keys and fails if any of them surfaces.

## The MCP server — start here, don't rediscover things

`tools/volt-mcp.js` is registered with Claude Code as **`volt`**. It exists so a session doesn't
begin by working out how to build the engine or which device index the nio is today. Ten tools:

- `volt_status` — git state + the BUILD ORDER from `ROADMAP.md`. **Call this first.**
- `volt_grep` / `volt_read` — get into `beta2.html` by line range. It is 716 KB in one inline
  script; never read it whole.
- `volt_test` — the suite, or one named suite.
- `volt_devices` / `volt_play` — every ASIO and WASAPI device the engine sees, and play two bars
  through one to measure its real latency. **Device indices change when hardware is plugged in —
  always list before playing, never trust a remembered index.**
- `volt_build` (engine / desktop / probe, `asio:true` for the ASIO backend), `volt_rust_test`.
- `volt_everything` — search Mickey's Everything index, same syntax as the Sample Search panel.
  Use it to find out what samples he actually has instead of speculating.
- `volt_vst` — scan his VST3 plugins, or load one and check it makes a sound. Same rule as
  `volt_everything`: find out what he owns rather than guessing. **SILENT is a normal answer** —
  it means the plugin wants a preset, not that it is broken.

Dependency-free on purpose: MCP is JSON-RPC over stdio, which is sixty lines of Node, and VOLT
does not take runtime dependencies. Add a tool by adding an entry to `TOOLS`.

## The engineering bible, and where it and VOLT actually diverge

`docs/daw-engineering-bible/` is a genuinely good reference and it has already absorbed this
project's own findings — `07` quotes our VST evidence back (main-thread editors, VST2 idle calls,
never unload the DLL). Use it. But it is a **target architecture**, and two divergences are real
enough that reading it as a description of VOLT would mislead:

- **VOLT's native engine does not own its graph.** `03`/`05`/`06` describe compiling a DAG,
  liveness-analysing buffers, publishing immutable graph generations and running a deadline-aware
  worker pool. The vendored `web-audio-api` crate owns all of that today; we write *nodes*, not a
  scheduler. So those chapters are only reachable by replacing that crate — a large decision, not a
  refactor. `01`'s "compatibility with current VOLT" note covers the browser but glosses this: the
  Rust crate hides its scheduling too. Treat those chapters as "what we would build if we ever own
  the graph", and the rest as binding now.
- **We violate one rule today, and it has already cost us.** `07` and `15` both say plugin scanning
  MUST happen in a disposable worker process, never by loading unknown binaries in the main app.
  VOLT scans in-process — which is exactly why Bionic Delay hung the desktop app inside
  `LoadLibrary` and forced the journal-and-blacklist workaround. The bible's rule would have
  prevented the failure outright rather than recovering from it. `vstprobe`/`editwin` already prove
  a separate process works; moving the app's scan into one is the real fix.

`15_ENGINE_DECISION_RULES.md` is a merge checklist for substantial *engine* changes. It is not a
gate on UI work or a one-line fix, and treating it as one would grind everything to a halt — but
for the mixer, the transport, or anything touching the audio callback, answer it.

## Rules of the house

1. **Test before you claim.** The suite is `node tests/run.js` (syntax check needs no deps and
   runs anywhere; browser suites need `npm i -D puppeteer-core` + Chrome). Add a test for
   anything worth keeping. If you assert timing or audio behaviour, **prove it by rendering
   audio** and measuring onsets — `tests/lib.js` has the analyser. Don't assert from data alone.
2. **Never commit or push unless he asks.** He reviews in the browser first.
3. **Don't break old songs.** Loading anything he saved before must keep working. Migration is
   automatic and on-read; export stays self-contained.
4. Keep it dependency-free at runtime. No CDNs, no frameworks. It has to work from `file://`.
5. Match the surrounding style: dense but commented, comments explain *why*, not *what*.

## Things that have burned us

- **Every letter key is a note.** `KEYMAP` covers `z s x d c v g b h n j m , l . ; /` and
  `q w e r t y u i o p` plus digits; `FXKEYS='rsudcgbtp'`. New hotkeys must use modifiers or
  punctuation that isn't mapped. `\` (transport mode) and `Ctrl+Space` (palette),
  `Ctrl+Shift+Space` (Vault) are taken.
- **Firefox eats `/` and `'`** for Quick Find. They're swallowed app-wide now — keep it that way.
- **Never bind Alt+F, E, V, S, T, H or B.** Those are Firefox's menu-bar mnemonics (File, Edit,
  View, History, Tools, Help, Bookmarks) and the page can't reliably cancel them — Alt+E and
  Alt+F silently did nothing in Firefox for months. `tests/bounce.test.js` fails if one comes
  back. Also avoid Ctrl+B / Ctrl+Shift+B (Firefox bookmarks).
- **Two keydown handlers.** The main one bails when a left-rail panel (`#side`) has focus; the
  transport/F-key one deliberately doesn't. Global keys go in the second.
- **Firefox has no File System Access API** and never will — `showDirectoryPicker` is
  Chrome/Edge only. Don't chase it; a Firefox add-on can't fix it either (extensions can't read
  arbitrary folders without a native-messaging host). OPFS + Export is the answer.
- **`file://` gives an opaque origin** (`location.origin === 'null'`). Broke postMessage once.
  OPFS still persists there — measured, both browsers.
- Native `dblclick` never fires on elements whose container re-renders on click (grid cells,
  instrument list). Count clicks in `mousedown`.
- Automation lanes are per-pattern but audio nodes remember values — pattern starts re-apply
  the base mix (`applyBaseAt`).
- WAV render needs a pre-roll or beat 1 renders cold.
- **Firefox's audio engine lies offline.** Its OfflineAudioContext renders a DynamicsCompressor
  as pure silence in small graphs, and its offline ScriptProcessor latency (96–160 ms, varies
  per run) says nothing about the live one (~29 ms). Never derive a LIVE timing figure from an
  offline render — measure in the live context (see `metLatMeasure`). Also: FF's compressor
  crushes short probe bursts to zero when two sit in series, and eats rectangular (DC) bursts;
  probe with tones. DynamicsCompressor lookahead itself is safe to assume: the spec fixes it at
  6 ms and both browsers measure exactly that.
- The mix is LATE relative to `schedRow`'s t: every note passes two compressors (12 ms) plus
  the master FX chain (Limiter L∞ ≈ 30–50 ms, browser-dependent, live-measured). Anything that
  must sound aligned with notes but doesn't go through the mix bus needs that delay added —
  the metronome does this via `MET.lat`/`metOut`.
- **Chrome's AudioParam does not follow the spec in two places**, both found by nulling the Rust
  engine against it and both worth knowing before you trust any sample-exact comparison. It reads
  an oscillator's frequency automation from the start of the 128-sample render quantum rather than
  the start of the note, so a note beginning mid-quantum runs at the *previous* frequency for its
  first few samples; and it begins a `setTargetAtTime` decay one block early, multiplying the
  parameter by `exp(-(T0-blockStart)/tc)` across the quantum before the event where the spec says
  hold. Both are inaudible (0.7 ms, 0.1 dB) and both wreck a strict null test. Firefox differs
  again — there is no single "the browser" to be exact against. `tests/parity.test.js` documents
  the measurements.
- **Three things about loading a VST3, each of which cost a crash to learn.** (1) **Never pass a
  null host context** to `IComponent::initialize`. The spec allows it; u-he Diva and Phase Plant
  access-violate on it, because they ask the context for `IHostApplication` and don't check the
  answer. `native/vsthost/src/host.rs` presents a minimal one and both now load. (2) **A `Plugin`
  must hold an `Arc<Module>`** — the plugin's destructor lives in the DLL, so the library cannot be
  allowed to unload first. Symptom: everything plays, the result prints, and the process never
  exits. (3) **Never unload a plugin DLL, and never call `ExitDll` while it stays mapped.**
  `FreeLibrary` on a plugin crashes; `ExitDll` without unloading leaves a half-dead module that
  breaks the *next* plugin to load. The library is leaked on purpose — one mapping per plugin file
  per session. Windows still unloads everything at process exit and some plugins fall over there,
  so anything that must close cleanly should end the process hard once teardown is done.
- **A plugin's window is a separate top-level window, made on the MAIN thread, and closing it
  HIDES it.** All three are load-bearing (`native/vsthost/src/editor.rs`). Not a child of VOLT's
  window: Tauri's loop is tao/winit's, and a loop running `IsDialogMessage`/`TranslateAccelerator`
  over a plugin's subtree eats Tab, arrows, Enter and Escape, so typing a preset name silently
  fails. Main thread because a window's messages only ever reach the thread that created it, and
  that is the only thread Tauri pumps — hence `run_on_main_thread` in every editor command. And
  hide-don't-destroy because **a plugin gets exactly one view for its whole life**: some crash on a
  second `createView`, so `Editor` is created once and only `vst_unload` destroys it. Teardown is
  `removed()` → `setFrame(null)` (the SDK sample has it backwards; JUCE and Ardour do it this way),
  `removed()` only if `attached()` actually succeeded, and `IPlugFrame::resizeView` needs a
  re-entrancy flag because the plugin calls it from inside its own `onSize`.
- **`kResultTrue == kResultOk == 0` on Windows, and `kResultFalse == 1`.** So `if (view.attached(…))`
  reads exactly backwards and a working plugin looks broken. Every check is `== kResultOk`.
- **`#[tauri::command]` RUNS ON THE MAIN UI THREAD. Slow ones must be `#[tauri::command(async)]`.**
  This is the single biggest freeze-the-app trap in the desktop build and it is invisible in the
  source: the macro's `execution_context` defaults to `Blocking`, so a plain sync command body
  executes on the thread that draws the window. Anything that opens a driver, scans plugins,
  instantiates one, or touches disk or network therefore freezes VOLT for exactly as long as it
  takes — and `(async)` on a *non-async* fn is exactly the fix, because the macro then runs the
  same synchronous body on the async runtime's thread pool ("sync_threadpool") instead.
  It cost a day: `vst_load` blocked for a second per plugin, so **pressing Play on a song with
  plugins froze the app**, and `vst_scan` blocked for a full 350-DLL scan at startup whenever the
  cache missed. Moving the work to a worker thread does not help on its own — the *command* still
  waits on the main thread. Verified in `tauri-macros-2.6.3/src/command/wrapper.rs:50`.
  **EVERY command is `(async)` now, including the fast ones** — leaving `eng_play` synchronous was
  still a freeze, because it takes the engine mutex and `eng_open` used to hold that mutex for the
  whole device open. Caught in the act with `cdb -pv` while the window was frozen: **zero CPU across
  all 44 threads** and the main thread parked in `NtWaitForAlertByThreadId`, which is a Rust lock
  wait — a deadlock, not slow work. `eng_open` now takes the lock twice, briefly, instead of once
  across the wait. **Diagnosing a freeze: measure CPU first.** Zero CPU means a lock; high CPU means
  a loop. `Get-Process` gives both, and `cdb.exe -pv -p <pid> -c "~*kn; qd"` (the Windows SDK's, at
  `C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\`) attaches without disturbing it.
  Because commands now run on a pool, anything the UI does not await can arrive out of order —
  `eng_mix` carries a sequence number so a stale snapshot is dropped rather than applied last.
- **A plugin that hangs while LOADING cannot be rescued, so it is remembered instead.**
  `plugins.rs` writes the plugin's path to `%LOCALAPPDATA%\VOLT\vst-loading.txt` (flushed) before
  opening it and erases it after; a leftover entry on the next launch means that plugin killed the
  last session, so it is blacklisted and never opened again. Without this, one bad plugin in a song
  wedges the plugin thread on **every** launch, for ever — Rust cannot kill a thread and the plugin
  is inside its own `LoadLibrary`. Same journal trick as the scan, for the same reason. A
  blacklisted plugin now fails with a message naming the file and saying how to un-blacklist it,
  rather than mysteriously.
- **Plugins live on their own thread (`native/desktop/src/plugins.rs`), and both other threads are
  wrong.** Creation, destruction and editor windows all happen there, and it runs its own Win32
  message pump. The audio thread is wrong because it has no pump (VST2 editors hang forever); the
  MAIN thread is wrong because instantiating a plugin takes about a second and a song with four of
  them freezes the window for four — which is literally what Mickey reported, *"the daw is just not
  responding"*, hours after the main-thread version shipped. **A slow plugin must block neither the
  UI nor the audio.** `01_ENGINE_ARCHITECTURE` in the bible already said this: the plugin host's
  instance manager and GUI bridge are their own domain, and the RT side does `process` and nothing
  else. It took two wrong threads to get there.
- **A VST2 plugin must be CREATED and DESTROYED on the same thread.** VST3 splits
  component (audio) from controller (UI), so it does not care; VST2 is one object whose editor goes
  through the same dispatcher as its audio, and it treats *the thread that created it* as its UI
  thread. Created on the audio thread — which has no message pump — FL Studio's plugins (Sytrus,
  Slicex) block inside `effEditOpen` **forever**, and the whole app goes unresponsive the moment
  you open the plugin window. Destroyed on the wrong thread, Sytrus exits 222 and Sylenth1
  access-violates. So `vst_load` creates on the plugin thread and hands the audio half to the audio
  thread; the audio thread never drops one, it posts it back down `volt_engine::vst::Retire` and the
  plugin thread destroys it. Measured 2026-08-11 with
  `native/vsthost/examples/editthread.rs`, which runs both threadings side by side — and note that
  **`editwin.rs` passed 50/50 while the app hung**, because it created and opened on one thread. A
  test that does not reproduce the threading proves nothing about the threading.
- **Rust block-buffers stdout when it is redirected**, so a harness that crashes loses everything
  it "printed" and appears to have died at the previous step. Flush after every line in any tool
  whose job is to say *where* a plugin died — this sent the Sylenth1 diagnosis down a wrong path
  for half an hour.
- **Sylenth1 crashes if it is the FIRST plugin loaded in a process**, in `Module::open`, before any
  editor. Load 129 VST3s first (as `vstprobe` and the app's scan both do) and it is fine. Not
  understood, not the editor's fault, and not something the app hits — but it will bite any new
  tool that opens one plugin in a bare process.
- **A VST2 editor with no `effEditIdle` is a frozen editor.** VST3 plugins run their own timers;
  VST2 was designed around the host poking the editor dozens of times a second, and without it
  meters do not move and some editors never finish painting. `editor.rs` drives it from a 30 ms
  `SetTimer` on the plugin window, which means the editor object has to sit at an address the
  window can hold — hence `Box::into_raw` into `GWLP_USERDATA` rather than living inside `Editor`,
  which gets moved into a registry the moment it is built. Kill the timer and null the pointer
  *before* freeing the box.
- **`examples/editwin.rs` is how the plugin-window code gets tested at all** — nothing in
  `tests/` can reach Win32, and driving VOLT's own UI is off the table. It runs the whole attach
  path against every instrument on the machine and never calls `show()`, so it is safe to run
  while Mickey is working. **50/50 instruments opened a window, 2026-08-11**, Kontakt and Phase
  Plant included. A plugin reporting 640×480 is echoing our default and probably never sized
  itself; its own numbers mean it really laid out in our window.
- **A plugin host has to kill its own process.** `volt_vsthost::hard_exit`. Everything releases
  cleanly and then Windows' DLL teardown runs plugin code that hangs (Jup-8000 V, forever) or
  access-violates (Vital, Diva, Kontakt). `std::process::exit` runs the same teardown; only
  `TerminateProcess` skips it. Flush first — nothing after it happens.
- **`ProcessData` must present EVERY audio bus the component declares — sidechains included.**
  `numInputs`/`numOutputs` are the component's bus counts, and the arrays need an
  `AudioBusBuffers` per bus even for buses the host has nothing to feed (honest silence + the
  silence flag). The host passed `numInputs:1` for years; WaveShell didn't care, but apQualizr2
  (apulSoft) declares a sidechain input and answered every block with SILENCE — in VOLT that
  read as "the plugin mutes the whole channel", and it was invisible until an fx chain finally
  gave it real input. Proven both ways with `fxprobe` (0.500 → 0.000, then 0.500 → 0.500 after
  the fix, 2026-08-13). When an effect passes the load but eats audio, probe it offline FIRST.
- **Cross-engine alignment is PER STACK — `nengLatCorr`.** ASIO's reported latency is real
  (measured playing at its own number) and goes into the classic sum; the shared Windows path's
  is fiction, and there the engine and the webview leave through the SAME stack, so their real
  latencies are near-equal and the correction is ZERO — ignore `NENG.lat` entirely on
  WASAPI/default. **Measured into existence by Mickey's ear**: the 35 ms he had to hand-nudge on
  2026-08-13 matched (browser `bLat` − engine reported) within a couple of ms, which is exactly
  what the equal-real-latency rule predicts. `NENG.nudge` (key `volt.neng.nudge2` — the old key
  is abandoned so that morning's hand-corrections don't double-apply) survives as a ±few-ms trim.
  Asserted in `nengine.test.js`: WASAPI start_in ignores NENG.lat, ASIO honours it. All of this
  dies when the engine owns every channel — one context, one clock — which is the PDC endgame.
- **A silent VST instrument is usually not a bug, and this was checked properly.** Kontakt, Phase
  Plant, Amigo and the Waves sampled instruments report *identical* capabilities to Vital and Diva
  — one event-in bus, a controller with hundreds or thousands of parameters, stereo out — and still
  output nothing. `IEditController`, `setBusArrangements` and a running `ProcessContext` are all
  implemented and changed none of them. They start empty by design (Phase Plant is modular; Kontakt
  has no library loaded) and need a preset, which means an editor window or a preset browser.
  `vstprobe <name>` prints the bus/parameter counts so this is checkable in one command instead of
  being guessed at again.
- **A VST3 file exports its controller class as well as its audio class, and they look identical.**
  Same name, same vendor, same `subCategories` (so `is_instrument()` says yes to both) — the only
  thing that tells them apart is `category`: `Audio Module Class` versus `Component Controller
  Class`. Filter on it or every plugin is listed **twice**, and the second copy cannot be loaded at
  all: it is not an `IComponent`, so `createInstance` returns E_NOINTERFACE (-2147467262) and
  picking the wrong Vital of the two is a coin flip. `vstprobe` had the filter from the start;
  `vst_scan_now` did not, and shipped duplicates for a day. Bump `scan_cache_path`'s number when a
  cached scan becomes *wrong* rather than merely old — nothing else ever discards it.
- **The VST3 folder has three layouts and Mickey has all three.** A plain DLL `Thing.vst3`, a bundle
  `Thing.vst3/Contents/x86_64-win/Thing.vst3`, and vendor subfolders containing either. Scan for the
  documented bundle form alone and half his plugins are invisible. Also: "is this an instrument" is
  not a flag — it is the substring `Instrument` in a free-text subcategory field, which is genuinely
  all the format offers.
- **Chrome's DynamicsCompressor deviates from the spec in three ways, and all three cost dB.**
  Found nulling the Rust channel strip against it (`native/engine/src/mixer.rs`, stage A in
  ROADMAP), all measured, none of them guessable: it **up-mixes a mono input to stereo**, which
  matters because the panner is next in VOLT's strip and a StereoPanner applies the *mono*
  equal-power law (-3 dB) to a mono input and unity to a stereo one — every VOLT instrument is
  mono, so this is 3 dB on every note; it is **not unity at ratio 1:1**, dipping ~2 dB for its
  first 90 ms before settling (after which it agrees with the spec algorithm to -87 dB); and when
  compressing it takes **1–2.5 dB more gain reduction** than the spec's algorithm. The lookahead
  itself is `floor(sample_rate * 0.006)` — 264 samples at 44.1 kHz, NOT 6 ms rounded up to a block.
  `tests/parity.test.js`'s `probe-comp-*` and `strip-*` jobs hold all of it still.
- **The vendored crate has forks in five files now, not one.** `native/vendor/web-audio-api`
  started as an ASIO fix in `io/cpal.rs`; `node/dynamics_compressor.rs` (sample-accurate lookahead)
  and `node/waveshaper.rs` (a curve may be reassigned, and cleared) joined it, then `io/mod.rs` +
  `media_devices/mod.rs` (the cheap `output_device_available`, 2026-08-12). Every edit is marked
  `VOLT FORK` — grep for it before assuming crate behaviour is upstream's. Its own test suite has
  **7 pre-existing failures that are not ours**: they open `samples/*.ogg`, which the vendored copy
  does not carry.
- **cpal's ASIO enumeration LOADS every ASIO driver to ask its name, and loading drivers flashes
  tray icons.** Never enumerate devices on a startup or open path — it was the taskbar light-show
  at every launch. Device ids are `HOST#hash` since 2026-08-12 (host in clear text, channel count
  no longer hashed), so the fork's id lookup opens only the named host and matches by NAME;
  `eng_open` takes the label as a parameter and refuses an id that does not resolve (the crate
  would otherwise silently fall back to the DEFAULT device — the wrong interface with a confident
  label). Enumeration belongs in exactly one place: the device picker, where listing is the point.
- **A dropped `AudioContext` KEEPS PLAYING — close it, never just drop it.** The vendored crate's
  `Drop` leaks the running backend on purpose (`// Continue playing the stream if the AudioContext
  goes out of scope`). For a script that is a convenience; for a DAW it is a **zombie engine**:
  every reopen — the buffer knob, a device change, the self-heal — left an orphan context still
  rendering the old graph, plugins and held notes included, that no command could ever address
  again. That is why **the panic could not kill the notes** (2026-08-13): it silenced the engine we
  still had a handle to while an orphan played on, and `eng_close` freed only the handle. The audio
  thread now ends every exit path with cut → all-notes-off → `ctx.close_sync()`, then signals
  `PREV_CLOSED`; `eng_open` waits (bounded, 2 s) for that word before it touches the hardware, so
  two engines can never hold one driver. The tell was in the log: **36,659 "underrun #1" lines**,
  duelling capacity monitors from contexts that were all supposed to be dead — monitors are
  generation-tagged (`STATS_GEN`) for the same reason.
- **A plugin instance does not survive a device reopen, so its PATCH has to.** Reopening builds a
  new audio thread and every plugin is created afresh, at its factory preset — which is the whole
  of *"i launch sytrus, put some preset there and after some time it reset"*. Before the old engine
  closes, each plugin is asked for its state (`VstNode::state` → `StateMsg` through the node port,
  answered on the render thread) and it is parked in `plugins.rs`'s `PATCHES`, keyed by owner
  (`i7`, `f0.1`) **and by plugin identity** so a slot that changed plugin cannot be handed the
  previous one's blob; `vst_load`/`fxvst_load` apply it before the plugin makes a sound.
  `Plugin::state`/`set_state` are VST3 `IComponent::getState/setState` (+ `setComponentState`, or
  the editor shows the old patch) and VST2 bank chunks. **Reading state allocates on the render
  thread** — only ever do it while a device is closing or the transport is stopped. This is in
  memory only: patches survive a reopen, NOT a restart. Putting them in the song needs a size story
  first (a Kontakt state is megabytes, the autosave goes to localStorage, and nothing may ever
  break saving).
- **Alt+P is a ladder, not a gain node.** stop → webview cuts → `eng_panic` (engine voices cut +
  a 128-note-off sweep into every plugin, ANSWERED within 800 ms) → no answer means wedged →
  `eng_close` drops the device (silence by disconnection) and `nengAutoOpen` brings it back. The
  same self-healing runs on any "audio thread has stopped": marked closed, reopened once per 30 s.
  And a plugin slot NEVER falls back to the instrument underneath it in the desktop build — a
  dropped engine used to replay Mickey's synth lines on the raw `sub` preset, which reads as the
  song being broken. Silent + said + self-healing, always.
- **The native engine now builds 13 of the 15 insert FX** (`native/engine/src/fx.rs`, stage D) and
  serves meters/spectra over `eng_meters` (polled, max-merged into the browser's own meters).
  `limit` and `pitch` are the two unported effects; a chain containing one skips it and says so
  via `neng:err`. FX *automation* still tunes only the browser's nodes — base values cross on
  every edit, automated sweeps do not.
- **VST effects live on mixer channels AND the master** (2026-08-12): `ts.fxVst` per track /
  `song.masterFxVst` → `fxvst_load` keyed by TRACK with **-1 = master**, hosted as a graph node
  with a real input (`Plugin::process_io`) between the ported chain and the drive (master: before
  the safety limiter). The scan lists effects now (`kind:"fx"`, cache `vst-scan-4.json` — bump it
  again if the shape changes); editor windows share the plugin thread registry at
  `FX_EDITOR_BASE+track`, master at `FX_EDITOR_BASE*2`. An instrument in the insert is refused at
  load ("no audio input") — it would mute the channel. `examples/fxprobe.rs` proves an effect by
  audio offline; WaveShell VST3 passes, Waves VST2 shells still refuse instancing. **A channel
  with `fxVst` silences its PORTED instruments' browser voices while the engine transport runs**
  (same mechanism as plugin slots; never in renders) — without that, the wet native copy played
  under the dry browser one and the FX "did nothing" on non-plugin channels.
- **STAGE THE HTML BEFORE YOU COMPILE, and never let the hash check tell you otherwise.**
  Tauri embeds `dist/` at COMPILE time, so `cargo build` then `node tools/mkdesktop.js` produces
  an exe wrapped around the PREVIOUS page — and the documented check (sha256 beta2.html vs
  dist/index.html) passes anyway, because staging is what made them match. It proves the two
  FILES agree, never that the BINARY contains either. A whole round of "the zoom does nothing,
  the menu still does nothing" on 2026-08-15 was this and only this. Either use
  `volt_build desktop` (it stages first) or run mkdesktop, THEN cargo, and verify the honest
  way: **the exe's mtime must be newer than dist/index.html's**. Grepping the exe for a string
  does not work either — the assets are compressed.
- **`dragDropEnabled` must stay FALSE in tauri.conf.json, and the config takes no comments.**
  With Tauri's OS drag-drop handler on, WebView2 installs its own `IDropTarget` and the page's
  HTML5 drag-and-drop dies completely: dragging a sliced instrument onto the grid showed the
  no-entry cursor, and dropping a WAV from Explorer fired no `drop` event either — VOLT listens
  for the DOM event and never for Tauri's, so BOTH halves were broken in the desktop build while
  working perfectly in a browser tab (2026-08-15). Off, the page owns every drop again, which is
  where all of them are handled (`dataTransfer.files` for audio, `text/plain` tags for
  instruments, slices and ideas). Only turn it back on together with a Tauri `onDragDropEvent`
  listener wired into those same paths. And do NOT try to leave a `"_comment"` key in that file
  explaining this — `tauri-build` rejects unknown fields and the build fails with a misleading
  "CLI version is newer than tauri-build".
- **THE distortion of 2026-08-14/15 was ONE line in the vendored crate, and it took a tap to
  find.** `param.rs` evaluated a FUTURE `setTargetAtTime`'s curve at the current block —
  `exp(+(T0−t)/τ)` — and poisoned the param's held value to f32::MAX. The engine schedules every
  lane cut ~160 ms ahead (lookahead), cut() uses τ = 2.5 ms, so **every retriggered native note
  played at full-scale saturation from the moment its cut was scheduled until it landed** — the
  safety limiter flattened that into a ±0.92 square at note pitch. Why it evaded everything:
  parity never schedules a far-future setTarget after a gap; the old cut/release unit tests only
  measured AFTER the event time — the exact region the bug spared; offline single-note renders
  never cut; and it only reached ears on channels whose native voices play in the ENGINE (VST-FX
  channels, ASIO-only) — browser-owned channels were clean, which made it look like "the VST
  distorts". Two `VOLT FORK` guards in param.rs fix it (hold the value before T0); the regression
  test asserts the WHOLE render (`a_scheduled_cut_does_not_deafen_the_note_before_it_lands`).
  Three lessons, earned expensively: **an audio bug is found by RECORDING THE AUDIO** (`VOLT_TAP=1`
  → tap.wav of exactly what the device is fed — the artifact named itself in one look after two
  days of clean meters); **a silence-region assertion must also bound the loud region** (peak-sane
  over the whole buffer, not rms-after); and **when every subsystem measures clean and the
  speakers disagree, the bug is between the measurements.**
- **Muting the webview is NOT the same as stopping it, and the difference is distortion.**
  `spkOut` hides the browser's mix; every voice behind it is still synthesised. `trig()`'s
  early-out used to test `fxVsts.length` — a hand-copy of half of `nengWantsTrack` — so once
  ASIO-only began sweeping every channel to the engine, the webview went on building oscillators
  for **the same notes the engine was already playing**, on every channel without an FX plugin.
  Two complete mixes on one machine, one of them inaudible: the ASIO callback starved on busy
  bars and it read as *"the snare fucks it up with distortions like crazy"* (2026-08-14). Fixed
  by calling `nengWantsTrack` itself. **Rule: any decision about who owns a channel goes through
  that one function — never re-derive it.** `nengine.test.js` counts `createOscillator` calls and
  fails if an engine-owned channel builds a single one.
- **Every channel has exactly ONE owner while the engine follows Play** (`nengWantsTrack`,
  2026-08-12): plugin channels and `fxVsts` channels sound from the ENGINE (their browser voices
  are silenced in `trig()`), everything else sounds from the WEBVIEW and its ported cells are NOT
  sent — "ASIO only" moves every ported channel to the engine too, **in BOTH transports since
  2026-08-14**: all fourteen instruments are ported, so the song-mode sweep has no holes, and
  `nengSpk` mutes in song mode by the same flag (the two are ONE decision — split them and a mode
  is silent or doubled). Song mode is still browser-CLOCKED (`nengSongStep` sends each pattern as
  the order turns over, re-aimed per pattern) — and in ASIO-only a LATE step send walks forward
  and joins rows late rather than skipping, because a skipped send there is a silent bar. One
  owner, never two. Anything that changes ownership mid-play (assign/remove an FX VST, toggle
  ASIO-only or follow) re-sends via `nengFollowTransport`; toggling follow OFF mid-play sends
  `eng_stop`, or the engine loops on under the webview.
- **VST effects are a CHAIN now** (`ts.fxVsts` / `song.masterFxVsts`, arrays in series,
  2026-08-12 night): `fxvst_*` commands all carry `pos` (Option — absent means 0), the engine
  keys `fx_vsts` by `(track, pos)` and re-splices the whole chain from the map on any change
  (`resplice_fx_chain` → `Mixer::set_plugin_chain`), editor windows key
  `FX_EDITOR_BASE + (track+1)*16 + pos`, param targets are `f{track}.{pos}`, automation lanes
  `vf<pos>.<id>` with a bare `vf.` meaning link 0. Old songs' single `fxVst`/`masterFxVst`
  migrate to link 0 on read (normTs / loadSong) and the old field is deleted — one source of
  truth. `nengFxSync` is a RECONCILER: the song arrays are the truth, every add/remove/song-load
  converges the engine through it (removing a middle link reloads the links behind it — plugin
  state is not in the song yet, so this is no lossier than any reload).
- **Plugin latency is COMPENSATED, not suffered** (PDC, 2026-08-12 night): every plugin reports
  its latency (`VstNode::latency` — VST3 `getLatencySamples`, VST2 `initialDelay`; L2 is
  thousands of samples, which is the "drums are late" Mickey heard). Nothing in the engine is
  live input, so cells are scheduled EARLY by channel-chain + master-chain (+ the instrument
  plugin's own) latency and the wet audio lands ON the row — `recompute_latency` keeps the sums,
  `max_shift` (capped 0.5 s) widens the clock's due() horizon, and a first row too close to Play
  clamps to "now" (late once rather than never). Duck triggers stay at musical time on purpose:
  the compensated audio arrives at musical time.
- **The engine never played a single ported instrument from the app until 2026-08-12**, and no
  one could hear it: `nengPattern` read `ins.p` — a field that has never existed (`.params` is
  the real one; `ITYPES[type].p` is the defaults) — so every native cell crossed with an empty
  bag and Rust's `Job::p()` refused it. Invisible because the webview played the same notes
  anyway; fatal the moment an `fxVst` channel silenced its webview copy (total silence, empty
  plugin meters — "no signal going through the vst"). The test that existed built instruments
  with a hand-planted `.p`, so it proved the bug instead of catching it. **A fixture must be
  shaped like the app's data or it tests nothing** — `nengine.test.js` builds via `mkInst`'s
  override argument now. Same sweep fixed velocity: the column is 0..64 (64=full, blank=.8,
  exactly `schedRow`'s reading); it crossed as `/127`, so every engine note — every plugin note
  since the beginning — played at half velocity.
- **Mute/solo reach the native mixer as a folded per-track bool** (2026-08-12): the browser mutes
  by not *scheduling*, which does nothing to a channel whose sound lives in the engine — muting a
  VST channel muted nothing. `nengMix` now sends `mute:!trackAudible(i), solo:false` per track
  (folded because the engine has no `polyLead`; a lane follows its leader), `Mixer::apply` gates
  the fader (`vg`), `toggleMS`/`clearMS` call `nengMix()`. Gate on the fader, not the notes:
  instant both ways, mid-note. Ducks still cross for muted tracks — the ghost-kick pump survives.
- **Plugin parameters automate** (2026-08-12): lane keys `vp<slot>.<pid>` / `vf.<pid>` → 
  `vst_param_set`/`fxvst_param_set` → `ParamMsg` through the node port → VST3 `IParameterChanges`
  (`vsthost/src/params.rs` — a COM object, same discipline as events.rs) or VST2 `setParameter`.
  Param lists are captured AT LOAD into `PARAM_LISTS` ("i{slot}"/"f{track}") — never ask a loaded
  plugin's controller from another thread. The editor knob does not follow (audio half only).
  **The gesture is right-click** (instrument row / VST FX slot), and the menu leads with the
  knob the user just turned in the plugin's own window: VST3 `performEdit` and VST2
  `audioMasterAutomate` feed a host-wide touch log (`last_touch()`, per-instance `tag`,
  `vst_touched` command). `performEdit` is NOT forwarded to the processor — plugins route their
  own GUI edits; a null handler proved it for months. Don't "fix" that into a double-apply.
- **The transport F-keys are BARE keys only** — Alt+F4 used to play a pattern instead of closing
  the app, because the handler ignored modifiers. Any modifier on an F-key falls through.
- **The window close is a question now** (`main.rs` intercepts CloseRequested → `volt:close-req` →
  the page answers with `app_quit`). Two rules keep it safe: the page's handler must ALWAYS end in
  `app_quit` or in staying open, and a second close request within 10 s bypasses the question — so
  scripted closes (the build loop's `CloseMainWindow`) still work: once for a clean page, twice
  ever. `EDITED` (session-scoped, set by `saveSoon`, cleared by `loadSong`) drives whether the
  question appears; the tab's `dirty` flag does not — the folder mirror clears that seconds after
  every edit, which would make the question never appear.
- **Esc hides the focused plugin window** — intercepted in `pump_messages` before dispatch, gated
  on the root window's class being `VOLTPluginWindow`. Plugins therefore never see Esc; that is
  the host convention, don't "fix" it.
- **Sidechain crosses as `Pattern.ducks`**, one trigger per note, separate from the playable cells
  on purpose: unported instruments (and someday muted ghost kicks) must pump the mix without being
  playable. `Mixer::duck` holds the per-channel config from `apply`; panic resets the duck gains.
- **`seedRnd` is not a textbook xorshift32.** `s>>17` in JS is an *arithmetic* shift — `>>` coerces
  to int32, so it sign-extends once the high bit is set. Any reimplementation (the Rust engine has
  one) must copy that, and must do the `/2^32` in double precision before narrowing to f32.
  Getting it wrong still gives perfectly good white noise, just not *the same* noise — inaudible,
  and instantly visible in a null test.
- Editing the file with CLI tools invalidates the Edit tool's state — Read a few lines first.
- **Git Bash rewrites `/flag` into a Windows path**, so `tasklist /fi …` becomes
  `'C:/Program Files/Git/fi'` and the command errors out. It cost a wrong answer once: a
  "wait until VOLT closes" loop built on it saw the error, matched nothing, and reported the app
  closed while it was still running and still holding the exe. **Check processes with PowerShell
  (`Get-Process volt-desktop`), not with `tasklist` from Bash.**
- `pointerup` blurs buttons and ranges globally. **Never remove it** — hotkeys die without it,
  because the grid's `mousedown` preventDefault blocks refocus.
- **Every F-key F1–F10 is bound**, and Ctrl+Shift+H is Firefox's history window. A new panel gets
  a toolbar button and a command-palette entry, not a hotkey. (Measured 2026-08-09: F1 help,
  F2 rename, F3 Vault, F4/F5 transport, F6 roll, F7 auto, F8 beat, F9 mixer, F10 recorder.)
  **Alt+A through Alt+Y are ALL bound** — Alt+Z (the Projects panel) was the last letter left.
  Grep for `Alt+[A-Za-z]` *and* for `altKey` near `Key[A-Z]` before claiming one is free: the
  first attempt at this took Alt+J and silently ate the automation lane's shape tool, because
  the capture-phase handler returned first. `tests/home.test.js` now fires all 25 other letters
  and fails if any of them reaches the panel.
- **Left-rail fold state is keyed by the section's class, not its index.** It used to be an array
  of positions in `volt.secs`, so inserting a section silently moved everyone's saved state onto
  the wrong panels. It's now an object in `volt.secs2` keyed by `sec-*`, migrated once from the
  old array. Add a rail section anywhere you like — but keep giving it a `sec-` class.
- **The menu bar references commands BY NAME — don't duplicate a function into it.** `MENUS` in
  beta2.html is a list of `COMMANDS` names; renaming a command makes the entry render greyed and
  labelled "(missing)" and `tests/menu.test.js` fails with the name. Never give a menu entry its own
  `f:` — one source of truth, or the menu quietly rots. And **no Alt accelerators ever**: Alt+F, E,
  V, S, T, H, B are Firefox's menu mnemonics (see below) and every letter key is a note.
- **A WAV render swaps the whole audio graph, and puts it back from a hand-written list.**
  `renderWav()` and the stem bounce set `AC` to an OfflineAudioContext, call `buildGraph()` on it,
  render, then restore about fifteen globals by name. **Add anything to `buildGraph()` and you must
  add it to BOTH save objects and BOTH restore lines**, or it silently keeps pointing at a node in
  a dead context. Three had already been missed (`metOut`, `loudAn`, `loudHP`), and `metOut` was
  fatal: `metClick()` then threw `InvalidAccessError: cannot connect to an AudioNode belonging to a
  different audio context` from inside `schedRow` → `pump` → `play`, so **with the metronome on,
  rendering a WAV meant Play threw and the song never started until reload.** Fixed 2026-08-10;
  `tests/render.test.js` fails by name if another one goes missing.
- **`spkOut` is the only thing between the mix and the speakers, and the only thing "ASIO only"
  touches.** It sits *after* the safety limiter on purpose, so the meters, the LUFS tap and the
  master recorder (all tapping `comp`) keep working while it is down, and a WAV render — which
  builds its own graph — cannot be muted by it. If you need to silence the webview for any reason,
  do it there and nowhere else. `nengSpk()` mutes in song mode too since 2026-08-14 — the engine
  is sent everything there now (see the one-owner bullet); before that it refused, because muting
  a mode the engine was not sent was total silence with the transport running.
- **`volt_build desktop` stages the HTML and enables ASIO — plain `cargo build` does neither.**
  Tauri embeds `dist/` at compile time and only `cargo tauri build` runs `tools/mkdesktop.js`, so a
  hand-rolled cargo build ships whatever `beta2.html` looked like when someone last remembered.
  `--features asio` is not a default either, and without it the app finds no ASIO devices and says
  nothing about why. Both fixed in the MCP tool; if you build by hand, do both yourself.
  **And do not trust the fix until this session has seen it work: MCP servers do NOT restart on
  /clear** (only when Claude Code itself restarts), so a session can spend a whole evening calling
  the old `volt_build` while the fixed one sits unread on disk — that is how the 2026-08-10 VST2
  build shipped with the previous night's HTML and greeted Mickey with "no VST3 instruments
  found". After any desktop build, `sha256sum beta2.html native/desktop/dist/index.html` must
  print one hash twice; two hashes means the exe is wrapped around a stale page.
- **The native master bus cannot null against the browser, and that is settled, not outstanding.**
  Its last node is a safety limiter, a limiter is a `DynamicsCompressor`, and Chrome's compressor is
  not the spec's algorithm (A2 in ROADMAP) — so every `master-*` parity job carries a documented
  gap instead of a null. The number that IS asserted is the one a mixer cares about: **the mix comes
  out within 0.5 dB.** Measured steady ratio 0.978, i.e. **-0.19 dB**, everywhere the limiter is not
  reacting. If a `master-*` job regresses, check the level line before the residue line — a residue
  that grows while the level holds is the limiter working harder (the delay repeats do this on
  purpose); a level that moves is a real routing or gain bug.
- **ASIO4ALL can wedge the audio thread INSIDE the driver, after the open "succeeded".** Seen
  2026-08-13: every plugin load "the AUDIO THREAD never answered in 30 s", window fine, one
  thread hot — stack (cdb) parked forever in `wdmaud!CUserModeSynth` teardown under
  `midiOutClose`, called from `asio4all64.dll`, AFTER `Opened` was already sent — so the badge
  was green, `SAMPLE_RATE` was set, and nothing worked. Same broken Windows-audio layer as the
  2026-08-12 zero-endpoints day; direct ASIO drivers (UMC, nio) bypass it. Defense shipped:
  **the device journal** (`%LOCALAPPDATA%\VOLT\dev-opening.txt`, same trick as vst-loading.txt) —
  written before every open, erased only at the audio thread's FIRST COMPLETED LOOP PASS
  ("audio thread: alive and pumping" in the log), because a successful-looking open proves
  nothing. An AUTO open (`eng_open{auto:true}` — startup and self-heal) of a journaled device
  refuses and the UI opens the picker; choosing it by hand always retries. Two more facts from
  the same morning: the poison SPREADS — once one open is wedged in wdmaud, every later open in
  any NEW process (any driver, WASAPI included) queues behind it kernel-side, so **kill every
  wedged instance before judging whether a device works** — the same device that wedged three
  times opened instantly once the corpses were gone. And the audio thread logs a breadcrumb per
  startup stage (context/latency/noise/mixer/mix/alive), so the log names the wedge point.
- **An ASIO driver can CRASH the app during enumeration, not just wedge it.** 2026-08-13 evening:
  WER access violations (0xc0000005) at `volt_desktop!ASIOInit`, twice, while the device picker
  listed drivers — the process died, no log, "its crushing". Symbolize a WER offset with
  `cdb -z <exe>` + local PDB and `ln volt_desktop+0x<offset>`. Defense: the enumeration journal
  (`dev-enum.txt`, third sibling of vst-loading/dev-opening) — written before a listing that
  will touch ASIO, erased when it survives; a leftover sets `VOLT_SKIP_ASIO`, which the fork's
  `all_hosts()` honours by not loading the ASIO host at all, and the picker grows a
  "⟳ Try ASIO drivers again" row. One bad driver day costs ASIO listing, never the app.
- **Zero WASAPI devices in `volt_devices` = WINDOWS is broken, not the fork.** Seen 2026-08-12:
  "the plugins not opening" turned out to be Windows' audio endpoint database completely empty —
  `EnumAudioEndpoints` returned 0 in every process (verified with a raw COM probe from
  PowerShell), the MMDevices registry had no Render children, drivers all CM_PROB_NONE, and
  restarting AudioEndpointBuilder/Audiosrv did not rebuild it (reboot required; suspects: Bluetooth
  headset churn, the Voice.ai virtual cable). Chain of symptoms in VOLT: saved device gone → engine
  never opens → `vst_load` never runs → every plugin window "REFUSED: no plugin on this
  instrument". Diagnose with `native/engine/examples/devdump.rs` (prints enumeration WITH the
  fork's log warnings) and a scratch cpal project before touching the fork. Direct ASIO drivers
  (UMC, Novation) bypass MMDevice and keep working; ASIO4ALL wraps the broken layer and may not.
  Since then, a missing remembered device AUTO-OPENS the picker instead of whispering in the
  status bar, and opening a plugin window with no engine says why and offers the picker.
- **VOLT is already fast — measure before optimising it.** Profiled 2026-08-09 on a clean load:
  pattern switch 3–9 ms even at 32×128, undo snapshot 0.4 ms, autosave 1.5 ms at *any* sample
  size (the OPFS hash store is why), one keystroke on an 8 MB song costs a 38 ms frame at worst,
  playback holds ~60 fps. The 90 ms switch quoted in `perf.test.js` is that suite hammering
  itself, not a real song. A switch does drift 3 → 9 ms with use — it's style/layout, not JS
  (a full `renderGrid()` resets it), and it isn't perceptible. `node tests/profile.js` re-checks
  all of it.

## How things fit together

- **Song data:** `song.patterns[i].data[track][row]` → `{n,i,v,c,x,g}` or `null`. `n===-1` is
  note-OFF. Note *length* is derived: a note runs until the next note, an OFF, or the pattern
  end. Polyphony is extra lanes (`polyLanes()`, `polyAddLane()`), max 32 tracks.
- **Timing:** `rowDur() = 60/bpm/lpb`. `song.lpb` (rows per beat) is editable and rescales every
  pattern losslessly. `Txx` is a sub-row delay of `x/192` of a row — that's how triplets and the
  arp's off-grid notes work.
- **Audio:** `trig()` returns a voice with `.release()/.cut()`. `pump()` runs on a 30 ms interval
  with 160 ms lookahead (1.6 s when the tab is hidden). `pump()` re-reads `SEQ.songMode` every
  row, which is why the transport can switch Song/Pattern mid-play without a restart.
- **Undo:** `snapState()` full JSON + `pushUndoSoon()` (250 ms debounce), capped at 24 MB.
  Samples are referenced by session id, not copied.
- **Samples:** stored once each in OPFS by SHA-256 (`ssPut/ssGet/ssGC`), namespaced by `VNS`.
  Autosave writes hashes; `loadSong()` resolves them; export still embeds base64 so a shared
  file stands alone. `ins._shaSrc` guards against a stale hash after a sample is edited.
- **Rendering:** `.prow` uses `content-visibility:auto` so off-screen rows are skipped. The
  `#grows{line-height:0}` / `.prow{line-height:normal}` pair and `calibrateRows()` exist to keep
  skipped rows exactly the same height as real ones. Don't remove them; the grid drifts.
  `styleIf()` skips redundant style writes in the 60 fps loop.

## Verifying UI work

Headless Chrome via puppeteer-core. Top-level `function`s and `let`s **are** reachable from
`page.evaluate` (shared lexical scope) but are **not** on `window`. `tests/lib.js` `open()`
boots a page and collects console errors. To test a dialog-free path, call the underlying
function (`loadSong(KITS['mine'].make())`) rather than the UI action (`loadKit('mine')` blocks
on a confirm).

## Deploying

Only when asked. The build files are tracked on `main` and GitHub Pages serves them directly —
commit and push is the whole deploy. Run the full suite first. The `dev` branch is stale; ignore it.
Cutting a new beta (`beta3.html`…) means: copy the current one, change `VOLT_NS` and the `<title>`
on line 5, add it to `EXPECT_NS` in `tests/store.test.js` and to the loop in `tests/syntax.test.js`,
point `FILE` in `tests/lib.js` at it, and leave the old one alone.
