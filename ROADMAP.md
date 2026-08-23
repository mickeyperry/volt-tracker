# VOLT roadmap

The running list of what's next. Tick things off as they land; add anything you think of.
Everything here is a suggestion until you say otherwise — nothing is committed to.

**New here?** Read `CLAUDE.md` first — house rules, architecture, and the traps.

**Where the work is:** `beta2.html` is the current build (its own storage, so it can't touch a song
saved in `beta.html` or the stable tracker). `beta.html` is frozen as the previous beta.

**How to use this:** open it any time (it's in the repo, so it's also readable on GitHub from
your phone). Or just ask Claude *"what's on the VOLT roadmap?"* and it'll read this file, and
update the ticks when it finishes something.

Status: `[ ]` not started · `[~]` in progress · `[x]` done · `[?]` needs a decision from Mickey

---

# WHERE THINGS STAND (2026-08-15, after the distortion hunt)

**The distortion is fixed.** It was one line in the vendored audio crate, not the instruments,
not the plugins, not the buffer — see the 2026-08-15 entry below for the full story and the
three lessons it cost. Along the way: no more zombie audio contexts, mono plugins no longer
throw away the right channel, patches no longer crash the app, the mixer no longer ignores
edits after a page reload, and song-mode pattern handovers no longer chop the tail off every
pattern.

**What Mickey should know is next**, in the order I'd do it:

1. **[ ] Plugin patches into the SONG.** They survive a device reopen today (in memory) but not
   a restart, so every session starts by setting the plugins up again — the thing he asked for
   twice. It needs a size story first: a Kontakt state is megabytes and the autosave goes to
   localStorage, so the patches want the SAMPLE STORE's design (OPFS by hash, hash in the JSON,
   embedded only on export) plus a per-plugin cap. Nothing here may ever be able to break saving.
2. **[ ] The engine owns song mode.** The browser still conducts: it sends each pattern as the
   order turns over, which is why the handover needed the fix below. Moving patterns+order+loop
   into `Cmd::Play` and letting `clock.rs` chain them removes the last cross-engine seam — and
   with it `nengLatCorr`, the nudge, and the "+web / ASIO only" split.
3. **[ ] The close-path deadlock.** Closing VOLT sometimes hangs with zero CPU (a lock, not a
   loop) and has to be force-killed. Evidence captured 2026-08-14; suspects are the open gate
   held across a slow driver open, or the exit patch-capture. Diagnose with `cdb -pv`.
4. **[ ] Scan plugins in a disposable process** (bible 07+15). Still the only thing standing
   between one bad plugin and the whole app.

**Also worth knowing:** `VOLT_TAP` / the palette's *"Record what the engine is playing"* writes
exactly what the soundcard receives to `tap.wav`. It is the tool that ended a two-day hunt after
every indirect measurement came back clean. Reach for it early next time.

---

# THE PLAN — make VOLT work like FL: one engine, flawless VSTs, zero hiccups (2026-08-13)

Written at Mickey's hold: *"do the roadmap so i clear here the shit."* Strike anything, reorder
anything — this is the priority order I believe in. The model to copy is the FL screenshots:
ONE device list, ONE buffer knob, plugins that never vanish and never glitch. Everything below
serves that.

### Plugin effects in a render — DONE 2026-08-23 ("i need it to be a regular daw")

Plugin INSTRUMENTS render since 2026-08-19 (`eng_render_vst`); effect CHAINS render now too
(`eng_render_fx`), so a render is the whole mix and the "these will be missing" dialog is gone.
The audio goes the other way, exactly as sketched: `renderWav` runs in PASSES —

1. **Stems** (only when chains exist): the same song scheduled again on a `2*K`-channel offline
   context; each chain channel's strip is cut at `fxOut` — post ported FX, pre drive, which is
   where the live engine splices the chain (the design question above, settled by mirroring the
   engine) — and tapped to its own channel pair. Plugin-instrument parts are rendered into the
   stems too, and cached so they cross the engine once.
2. Each stem crosses to `eng_render_fx` as the invoke's RAW BODY
   (`[u32 len][json][pad][f32 stereo]` — a JSON array of 4M samples is minutes of serialize).
   The engine builds fresh instances of the chain at the render's rate, with the session's
   patches (`f{track}.{pos}`, flushed force:true first), DRIVEN — song tempo, beat-0 anchor —
   and returns a WAV trimmed by the chain's summed latency, so the wet stem lands exactly where
   the dry one was.
3. **The render**: the dry tail is disconnected and the wet stem plays into the drive — REPLACE,
   never sum (two copies is phasing; `fxrender.test.js` pins unity-wet == no-chain level). A
   chain that fails ships that channel DRY and the status line names it.
4. **The master chain** gets the SUMMED mix: the pass stops at the ported master chain's exit
   (pre safety limiter, where the engine splices it live), the engine wets the mix, and a final
   tiny pass runs it through the limiter (`masterFxChanged` honoured, so an own Limiter still
   parks the safety net).

Also fixed on the way: render-scheduled automation lanes no longer sweep the LIVE plugins' knobs
(`applyVstAuto` bails while rendering — before, a render twiddled real parameters and the forced
patch flush could save them). Still outside a render: `vf`/`vp` plugin-param automation lanes
(the offline instances aren't fed lane values yet), and `renderBounce` (bounce-to-sample) still
knows nothing about plugins — both are follow-ups, not regressions.

**The leftovers, closed (2026-08-23, "fix all the stuff"):** the two follow-ups above are done.
**`vp`/`vf` plugin automation lanes reach renders now** — a render's lane values are RECORDED at
their row times (`VSTRAUTO`, armed by renderWav; `applyVstAuto` was already refusing to sweep the
live knobs) and travel with the offline jobs; `ParamMsg` grew a `when` and the processor parks
timed values until their block, exactly like notes (`set_param_at`). Block-rate, the same
precision the live path has. And **a bounce carries plugins now** — `renderBounce` runs the same
stems/parts machinery (a Sytrus channel used to bounce to SILENCE; a chain channel bounced dry):
parts and chains bake into the sample like the ported FX always have, the master chain only for
a mix bounce (a channel bounce keeps the master neutral, same as `masterFx`). A bounce that
starts mid-pattern anchors the plugins' bar position at its first row (`beat0` in both job
shapes) so synced LFOs phase-lock to the right place. FREEZE therefore stashes `ts.fxVsts` along
with the strip — baked into the frozen sample AND still live would be double-processing — and
puts it back on thaw, `nengFxSync` converging the engine both ways; the stash happens after
freeze's level-check bounce, which must still carry the chain so both sides of the comparison
match.

**"The audio delays when moving from pattern to pattern while playing songs" (2026-08-23) —
found and fixed.** The song-mode turnover send (`nengSongStep`) fires when pump BOOKS the
boundary — ~160 ms of budget — and spent it on two slow things: `Array.from(pcm)` turned every
first-time sample upload into a multi-megabyte JSON stringify on the main thread, and the
uploads + `nengVstSync` were awaited BEFORE `eng_play`, so on a first pass through a
sample-heavy song the engine's copy arrived after the seam and joined rows late — the delay is
the engine "walking forward" (late by design beats absent, but late was every first pass). Two
fixes, both structural: **samples cross as raw bytes now** (`eng_load_sample_raw`,
`[u32 len][json][pad][f32le]` — the same body shape as eng_render_fx; the JSON command survives
only for the contract test), and **each pattern's samples are PREFETCHED a whole pattern early**
(`nengWarmPattern` — fired for order[n+1] right after order[n]'s handover send, and for
order[1] right after Play's first send; NENG_SENT makes every warm after the first free). The
boundary send is serialize + one invoke now, and the engine's parked-handover seam gets its
pattern with a full pattern of margin instead of a starved 160 ms. Contract updated in
`nengine.test.js` (raw body unpacked and asserted) and pinned in Rust
(`the_raw_sample_body_parses_byte_for_byte`).

**Review round, same night (asked for and done):** four findings from re-reading the code and
Mickey's real render logs. (1) **A chord was four plugins** — `vstrTake` keyed parts per poly
LANE, so full-on's 6 Sytrus chords became FOUR identical 22.7 s engine renders (the log showed
them) and four instances where live has one: wrong for unison/mono/shared-LFO patches and 4x the
load time. Parts key by `polyLead` now — one part per channel, like the live topology; distinct
channels keep distinct instances on purpose. (2) The stems pass ran all chain channels in one
context, and an OfflineAudioContext caps at 32 channels — 17+ chain channels would have thrown
and killed the whole render; stems come out in batches of 16 now, the part cache keeping batches
from re-rendering instruments. (3) Muted (or not-soloed) chain channels paid a full chain render
for a silent stem — skipped now, same audibility fold schedRow uses. (4) The app stays
interactive between the passes, so an edit mid-render puts wet audio under the wrong notes — a
signature over everything scheduling reads is checked after the last pass, and the status line
says "EDITED while this rendered — render again" when it moved (re-rendering automatically would
loop forever under a live jam; saying so doesn't). `fxrender.test.js` grew the dnb8 shape as a
check: a plugin instrument ON a chain channel reaches the stem, comes back wet at unity level
(not doubled), crosses the engine once per render, and a muted chain channel asks nothing.

This section shipped the same night as — and because of — **"the renders come out with phasing"
(2026-08-23)**. The hunt, so the reasoning isn't re-argued: the browser's own render path
measured CLEAN first (a lone-noise-hat render through `renderWav()` autocorrelates at the noise
floor, 0.068, bare and with the Limiter L∞ and Soft Clip — no doubled copy, no dry-path leak;
rendering full-on.volt headlessly showed nothing above tonal periodicity). The real causes were
both at the plugin seam: **(1) `eng_render_vst`'s fresh instance was an UNDRIVEN PROBE** — the
live path sends `set_tempo` + `set_transport` on every attach, the render path sent neither, so
every rendered plugin part ran its synced LFOs/gates/arps/delays at the context default (120,
stopped — `win.rs` ~347) against a 145 (full-on) or 174 (dnb8) bpm song, in the WAV and nowhere
else; fixed by driving every offline instance (`bpm` + `start` in the job, negative-beat pre-roll
trimmed), pinned in `tests/vstrender.test.js`. **(2) The missing effect chains themselves** —
dnb8 2 2 carries nine across seven channels (apQualizr2 on the bass, IL Stereo Enhancer, Love
Philter), full-on seven across three (RBass on the bass): every render shipped without its
low-end chains and wideners, a different mix than the engine plays live. That is what the passes
above fix.

**What "phasing in a render" means AFTER all of the above (2026-08-23, measured on zaza.wav):**
his own rendered file scans CLEAN — no doubled copy at any lag (the only autocorrelation peaks
are the bass pitch periods), L/R aligned to the sample, levels sane. What remains audible is the
STRUCTURAL difference: a render is made by the BROWSER engine while he listens to the NATIVE one,
and on the saw-family (reese, supersaw, psybass) the two differ by the documented WAVEFORMS
phase gap (-27 dB residual, "no port however correct can null that") plus Chrome's compressor
quirks — same notes, same levels, a different interference texture, which on a reese-driven mix
reads as "the render phases". Shipped tonight: renders and bounces run at the DEVICE'S sample
rate (`SR=(TAURI&&NENG.rate)||44100`) so the render's plugin instances match the live ones
exactly and playback doesn't resample; a browser tab keeps 44.1. **The real end of this class is
the P3 flip's render half: the ENGINE renders the WAV** — it already has all 14 instruments, the
strips, the buses, 13/15 fx and the plugins; a render made by the engine that plays IS the mix,
by construction. That is the next big block of engine work, not an evening patch.

**A picked device could lose to the self-heal (2026-08-23, "when i change the asio 4 all it gets
back to wasapi") — fixed.** ASIO4ALL sat ~14 s inside `eng_open` while the old thread was already
dead; that window looked exactly like "the engine stopped", `nengFailed` → `nengReopenSoon` fired
`nengAutoOpen` with the REMEMBERED id, and the queued reopen landed right after the ASIO open and
replaced it (log: `alive and pumping (ASIO#…)` and `closing the device` in the same second). Two
belts now: `NENG.opening` holds the self-heal off while ANY hand-initiated open is in flight
(picker, buffer reopen, assistant reopen), and the picker REMEMBERS the choice before opening —
a race that still slips through reopens the device the user picked, not the old one (an open
that fails puts the previous device back so the next launch doesn't chase a broken one).

## EVERY PC — "make the program stable so it will work on every pc even the shitty ones"

Asked for 2026-08-23. VOLT is fast on THIS machine (profiled: switch 3–9 ms, ~60 fps playing);
"every PC" is a different discipline — assume a 2-core laptop with 4 GB, integrated graphics, no
audio interface, Windows default output, and someone else's plugin folder. The plan, in the
order it pays:

1. **Low-spec mode, automatic.** The fps guard (perf.test's meter) and the engine's underrun
   counter already measure the two things that hurt; when either degrades for more than a few
   seconds, shed load in order of invisibility: meter/spectrum poll 30 Hz → 10 Hz, LUFS off,
   playhead glow off, autosave debounce widened. A palette toggle forces it; the status bar says
   it engaged. Everything shed is cosmetic — the transport and the mix are never degraded.
2. **Underruns answer themselves.** The counter exists and the status line says "a bigger buffer
   cures this" — make it a one-click: after N underruns in a minute, offer (or in low-spec mode,
   just do) one buffer step up, said in the status bar. FL's exact move.
3. **The 160 ms cushion scales to the machine.** pump()'s lookahead is sized for a steady main
   thread; a shitty PC's main thread is never steady. Measure tick lateness (the scheduler
   already knows when it ran vs when it meant to) and widen LOOK toward the hidden-tab 1.6 s as
   jitter grows — the cost is only edit-to-ear latency, which beats gaps.
4. **First-run honesty.** No interface, no plugins, unknown WebView2: the first launch should
   say what it found (default output at N ms, no ASIO — fine; X plugins) instead of assuming
   this machine. The device journal + worker scan already make the dangerous parts safe.
5. **Memory ceilings audited** for 4 GB machines: the undo cap (24 MB) and sample store are
   bounded; the render passes are the new heavy path (stems = full-length buffers per chain
   batch) — cap concurrent stem buffers by total bytes, render in smaller batches when
   `navigator.deviceMemory` is small.
6. **A test that IMPERSONATES the shitty PC**: run the perf suite with CPU throttling
   (puppeteer's `emulateCPUThrottling(4x)`) so "works on a weak machine" is a number in CI, not
   a hope. This is the item that keeps the rest honest.

None of this is speculative machinery: every item hangs off a measurement that already exists
(fps guard, underrun counter, tick clock, profile.js). Next session's block, alongside the
engine-render project above.

**The generators (2026-08-23, "anything else we can add for the fun"):** two palette commands
that WRITE NOTES, so they work identically in both engines and cannot touch audio code —
**"Generate: build-up"** (the psytrance riser as tracker data: 8ths → 16ths → Rxx bursts
doubling into the last row, velocity climbing 16→64, and a `fltF` cutoff lane rising under it;
no selection means the LAST 16 rows, because a build-up leads into the next pattern) and
**"Generate: acid line"** (a 303 pattern in phrygian off the channel's own most-played root —
root-heavy with scale stabs, octave jumps, accents, `Gxx` slides, and rests; different every
run). Both overwrite only the cursor channel's region, both undo with one Ctrl+Z, both reroll by
running again. Shared scope logic in `genScope` (selection rows / channel's own instrument and
root). Tested in `tests/randomize.test.js` — region bounds, climbing velocity and lane,
scale-membership, the accented root on row one.

**Four more, same night ("give me more of those"), same rules, all in Track → Generate and the
palette:** **rolling bass** (the psy engine room — every 16th except the downbeat, which is left
EMPTY for the kick; root throughout, a reach to the 3rd/5th at phrase ends), **drum fill** (last
8 rows by default, pitches only from what the channel already plays — randomizeNotes' no-ghost
rule — density and velocity climbing, Rxx stutters, always ends on a full hit), **groove stamp**
(the one that shapes what is THERE: velocities only — accents on the beat, a push at the half,
ghosts off it, human waver; poly lanes included), and **chord stamp** (asks the flavour — fifth,
octave, power, minor, sus2 — then grows poly lanes as needed and stacks every note in the
region; glides copied so the stack bends together, note-OFFs copied so it STOPS together). All
tested in randomize.test.js (41 checks): kick-row emptiness, pool membership, untouched
pitches, lane growth and the stack-wide OFF.

**The CPU meter (2026-08-23, "can we see cpu usage of the program itself?"):** a readout beside
the ASIO badge — `CPU 12% · DSP 6%`, green under half, amber under 80, red past it. CPU is the
WHOLE app against the whole machine, Task-Manager style: `cpu.rs` walks the app's own process
tree (toolhelp snapshot + parent links) because WebView2 renders the entire UI in separate
processes — this process alone would show the engine and quietly omit the half the user looks
at. DSP is the render callback's share of its block budget from the existing `eng_stats`, the
number that turns into crackle past 100. Stamped at poll time so the CPU half ticks even with no
device open; the eng_stats poll no longer gates on NENG.open for the same reason. Both feed the
low-spec-mode plan above — the meter is its trigger measurement made visible. Unit-tested
(`cpu::tests::the_meter_reads_its_own_burn`: first poll refuses to invent a number, a deliberate
120 ms burn reads back as a sane percent).

## P1 — STOP THE HICCUPS (the distortion that follows the notes)

- [x] **Zero allocations in the audio callback** (2026-08-13 — proven clean offline, deployed).
- [x] **A buffer-size setting, exactly FL's** (2026-08-13 night — a ⚙ row in the device picker:
      Automatic / 1.5–46 ms shown with the sample count, persisted as `volt.neng.buf`, sent as the
      latency hint on every open, applied immediately by reopening the device. Plus FL's other
      half: an UNDERRUN COUNTER — `eng_stats` reads the render thread's load and underrun events,
      the ASIO badge wears `N⚠` when hiccups happen, the tooltip shows avg/peak load, the log
      gets one line per event so a crackle has a timestamp.)
- [ ] **Audio-thread audit against the engineering bible** (docs/daw-engineering-bible 01+03):
      no locks/waits in the callback, the tick loop must sleep when busy (check for a hot spin),
      report render load so "the machine is overloaded" is a number in the log, not a guess.
- [ ] **One Audio Settings dialog like FL's**: device + buffer + sample rate + measured latency
      in one place, instead of picker + palette toggles scattered.

## P2 — VSTS THAT NEVER VANISH AND SURVIVE EVERYTHING

- [x] **Same fix the FX chains got, for INSTRUMENT plugins** (2026-08-13 night — and the REAL
      vanish was worse than the load-order race: `snapState()` never captured `ins.vst`, so EVERY
      undo rebuilt the instruments without their plugins and the save 700 ms later made it
      permanent. Fixed all four legs: vst + masterFxVsts in the undo snapshot, restored by
      applyState, `nengVstSync` is a true reconciler now (unloads what the song stopped claiming
      — delete/replace/clean converge too), and `nengPickVst` writes the song BEFORE the slow
      load. `nengine.test.js` fails by name if any leg regresses.)
- [~] **Plugin STATE** (VST3 component state / VST2 chunks). **Half done 2026-08-13 night:**
      `Plugin::state`/`set_state` exist, and a patch now survives the thing that was actually
      eating it — a device reopen (`PATCHES` in plugins.rs, captured as the old engine closes,
      restored before the new instance plays). Still to do: **into the SONG**, so it survives a
      restart. Needs a size story first — a Kontakt state is megabytes and the autosave goes to
      localStorage, so patches want the sample store's treatment (OPFS by hash, hashes in the
      JSON, embedded only on export) and a cap. It must never be the thing that breaks saving.
- [~] **Scan and load in a DISPOSABLE WORKER PROCESS** (bible 07+15 both call this mandatory).
      **The SCAN half is DONE, 2026-08-23** — and it convicted a plugin on its very first run.
      `vst_rescan` spawns this same exe with `--scan-worker` (decided before the Tauri builder,
      or single-instance would treat the worker as a second VOLT); the child scans, writes the
      cache, hard-exits. Three attempts with a 120 s deadline each: a dead or hung child is
      killed, its own journal names the file it was inside, and the next child blacklists it
      and walks past — the old self-healing, minus the part where the app died too. First real
      run: attempt 1 WEDGED inside `mda Detune.64.dll` (0 CPU, parked — the exact Bionic-Delay
      failure), attempt 2 blacklisted it and scanned 920 plugins in 36 s. In-process scanning
      survives only as the fallback for "could not even spawn a worker". LOADING is still
      in-process (that is full plugin sandboxing — audio streaming between processes, a
      different project); its journal now tells a deliberate quit from a wedge, below.
      **Also fixed, same session: a deliberate quit no longer blacklists innocent plugins.**
      Closing VOLT while a song's plugins were still loading left `vst-loading.txt` behind, and
      the next launch read it as "this plugin killed us" — that is how Sytrus and WaveShell got
      banned mid-evening. Every intended exit (`app_quit`, the 10 s escape hatch) writes a
      `clean-exit.txt` marker now; at the next launch, a journal from a cleanly-quit session
      whose load was under 25 s old is an interrupted load, not a wedge (older than 25 s — past
      the app's own 30 s patience — is still blamed, so a genuinely wedged plugin the user quit
      around is still caught; a stale marker cannot acquit a newer journal — the mtime
      arithmetic fails closed). `journal_blames` is unit-tested in plugins.rs.
- [ ] Handle latency CHANGES live (VST3 kLatencyChanged) so PDC follows linear-phase toggles.

## P3 — ONE ENGINE (the FL model, and the end of every alignment/ownership trap)

**DECIDED 2026-08-14** (*"no seperation bs and make it stable like a motherfucker"*): desktop
goes one-engine — Rust plays everything, webview draws. The web version stays alive with its
Web Audio engine; only desktop stops using it for sound.

- [x] **Port the remaining instruments** — **DONE 2026-08-14, ALL FOURTEEN are in the engine.**
      supersaw, psybass, pluck, acid303, mutant, glitch ported in one push (sliced was already
      in, waiting only for the app to cross its slice params — it does now). Every one proven by
      null test, not by ear: psybass -53.7 dB rms, glitch FM mode -74.3, acid303 -44.6/-41.8,
      mutant -27.1, supersaw -27.2, pluck -28.2 — levels within a third of a dB everywhere.
      The saw-family bars sit on the documented WAVEFORMS table gap, same as reese always has.
      Two port lessons worth keeping: glitch's "random" IS testable because the seed folds
      note+vel through JS's ToInt32 (ported bit-exactly, rnd() call order and all — src 0 nulls
      at -53 dB *with* chaos on), and its oscillator modes can only be nulled with grains on
      quantum boundaries — Chrome quantises mid-quantum frequency changes to its 128-sample
      blocks, so jittered grains decorrelate in phase while matching in level. That is Chrome
      being Chrome (the QUANTUM note in parity.test.js), not a port error.
      **Consequence, live today: "ASIO only" is now the COMPLETE one-engine experience in BOTH
      transports** — every channel, every instrument, pattern AND song mode, out of the chosen
      device, no holes. (Same evening: nengWantsTrack sends the sweep in song mode, nengSpk mutes
      there by the same flag, and a late nengSongStep send in only-mode joins the bar rows late
      instead of skipping it — late by a row beats absent by a bar.) Song mode is still
      browser-clocked, re-aimed each pattern; the engine-owned song transport below is what ends
      that too.
- [ ] **Engine plays EVERYTHING, always** (song mode included), webview becomes UI only:
      one clock, one master, exact PDC everywhere, no "+web"/"ASIO only" modes to explain.
      This deletes: cross-engine alignment, the ownership rules, the split master, nengSpk.
      **The concrete next step (all instruments are ported, so this is now unblocked):**
      teach the ENGINE song mode — `Cmd::Play` grows a song shape (patterns + order + loop
      point), `clock.rs` chains patterns of differing lengths, `neng:row` reports (ord, row),
      `route_vsts`/ducks re-derive per pattern, PDC horizon spans boundaries. Then extend
      "ASIO only" to song mode, live with it, and only then flip the default and delete the
      modes. Staged on purpose: each step leaves VOLT playable, and the flip happens after the
      engine transport has been trusted for a while, not the same night it was written.
- [ ] Remaining tracker commands in the engine: **Rxx retrigger, Pxx pan**, painted `p.*` lanes,
      live FX-automation sweeps (base values already cross).
- [ ] Mix LFOs engine-side.

## P4 — POLISH THAT MAKES IT FEEL PRO

- [ ] Freeze: one-click "freeze + keep FX live" flow (it already bounces browser-perfect audio).
- [ ] The reese saw-table gap at low notes (−35 dB vs Chrome): port Chrome's octave-crossfade
      into the vendored oscillator so saws null like everything else.
- [ ] apQualizr2-class plugins: standalone sidechain ROUTING (feed another channel into bus 2).
- [ ] Assist channel: more ops as needed (it already reads state and edits the mix remotely).

---

## 2026-08-16 — VOLT builds for Linux and macOS

The whole porting surface is pushed DOWN into `volt-vsthost` so nothing upstream carries a `cfg`:
`win.rs` is the real (Windows) implementation, `portable.rs` is its honest twin, and `lib.rs` is
the shared root — plugin descriptions, the blacklist, the touch log, `SCAN_SKIP`, `hard_exit`,
and a new `volt_data_dir()` (`%LOCALAPPDATA%\VOLT` / `$XDG_DATA_HOME` / `~/Library/Application
Support`). The desktop crate's every journal, log, scan cache, patch file and tap goes through one
helper; `log.rs` gets local time from `GetLocalTime` or `localtime_r` and stays dependency-free.
`native/engine` needed NO changes — it compiles against either half.

**Verified on the live tree, not just claimed:** Windows `cargo check --features asio` clean,
vsthost 21/21, engine 31/31, desktop 18/18, JS 1077/1077, the installer rebuilt and VOLT relaunched
with its data folder untouched. And the honest proof the facade is real:
`cargo check --target x86_64-unknown-linux-gnu --all-targets` in vsthost **compiles on this
Windows machine** — lib, vstprobe and every example, because off Windows the crate is std-only.

**What a Linux/macOS user gets:** the native engine — all fourteen instruments, thirteen insert
FX, mixer, sidechain, PDC — over ALSA/CoreAudio through the vendored cpal fork. **No VST hosting**
(the pickers come back empty and a load says why) and **no Everything search**.

**Build:** Linux `apt install libasound2-dev pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev`, then
`node tools/mkdesktop.js && cd native/desktop && cargo tauri build --bundles deb,appimage`
(no `--features asio` — that is Windows-only). macOS: Xcode CLT, then `--bundles app,dmg`.
Both want an icon in their format (`icon.png` / `.icns`) or `--no-bundle` for a raw binary.
The engine alone builds anywhere: `cargo build --release` in `native/engine`.

**Not done:** neither OS can be compiled from this machine (alsa-sys wants ALSA headers, Tauri
wants webkit2gtk / macOS frameworks), so the first real Linux/Mac binary needs one of those
machines. `tauri.conf.json` still bundles NSIS only.

## 2026-08-16 — Cool mode, with a switch for every part

- **⚙ Settings has a Cool mode section**: a master switch plus one tick box per part — panels &
  dialogs pop, buttons press, status messages rise, the tip on start-up — and an all/none link.
  Asked for as "nice little animations for the fun of using the daw", then immediately, and
  rightly, "make it togglable so we can turn off cool mode or parts out of it".
- **Each part is a class on `<body>`** (`cool-pop`, `cool-press`…) and every animation rule
  requires its class, so switching one off removes a class and nothing else happens: no
  re-render, no animation left half-played. The two JS-side parts check their flag before doing
  any work — the status pop skips its forced reflow entirely when off, and clears the class, so
  turning it back on cannot replay a stale one (caught by the headless check, not by eye).
- **`prefers-reduced-motion` still wins over all of it.** A system-wide accessibility choice is
  not ours to override with an app setting.
- Also in the palette ("Cool mode on / off") and F1. Settings persist in `volt.cool`, and the
  per-part choices survive the master switch going off and on.

## 2026-08-16 — the mouse selects in FIELDS, and the row-0 line was under the header

- **Drag-selection snaps to whole fields now** — note | instrument | volume | fx — instead of
  crawling across all eight sub-columns ("the snapping of the selection is delaying through all
  the spaces between the actual values"). Each end of the drag opens outward to the edge of its
  own field (`FIELD_SPAN`), so a drag from an instrument digit to a volume digit takes both
  fields whole. The eight sub-columns still exist for the KEYBOARD — typing hex needs a digit
  cursor, and Shift+arrows still move by single columns. Cross-channel drags snap to whole
  channels as before. Verified headlessly: fields 1–4 from a c2→c3 drag, outward both ways,
  channels unchanged.
- **The row-0 selection line was hidden UNDER the sticky header**, not clipped at the top as
  first assumed (fixing that assumption did nothing — his screenshot proved it). Measured: rows
  start at offsetTop 60, the frame drew its top edge at 59, and the header paints over 0..60.
  The frame no longer rises above the first row: at row 0 the line draws inside the row's top
  pixel. The general lesson: verify a fix against the REPORTED symptom, not against the theory.

## 2026-08-15 (night, round 14) — six of Mickey's findings

1. **Piano roll Ctrl+A now selects every poly lane.** `prNotes()` walked all lanes but the
   select-all dropped `t` from each entry, and `prIsSel` defaulted a missing lane to the cursor's
   — so on an Alt+Y chord track it visibly selected only the lead lane and every operation after
   missed the rest of the chord. One line: keep the lane.
2. **Ctrl+S saves, on desktop, into the songs folder.** It used to fall into the browser-tab
   download path (VAULT.proj is FS-access, a browser thing), which read as "is there a way to
   save at all?" — it now flushes and writes via `nstoreWrite` and says exactly where.
3. **Ctrl+Alt+M = the MASTER.** Track panel flips to the master (fader, EQ, master FX, plugin
   chain), mixer opens if closed, press again to flip back. Also in the palette and F1.
4. **Space works inside a plugin window** — swallowed by `pump_messages` (like Esc), forwarded as
   `volt:vst-space`, and the page runs the exact armed-mode Space logic. The trade, documented:
   a space cannot be typed into a plugin's own text fields.
5. **A selection reaching row 0 keeps its top line.** The frame is drawn 1px outside the rows,
   which at the top put it at -1 — above the scroll content, clipped. Clamped.
6. **Panic now actually silences VST instruments.** The reset's `flush_state` was a LITERAL NO-OP
   for VST2 (empty body — suspend/resume now) and only `setProcessing(0)/(1)` for VST3, which
   most synths ignore as a voice-kill; the spec hangs buffer/voice flushing on DEactivation, so
   it is the full `setProcessing(0) → setActive(0) → setActive(1) → setProcessing(1)` cycle now.
   Watch: this also runs on a FRESH Play (same reset path) — if Play ever feels sluggish with a
   giant sampler loaded, split the panic flush from the play flush.

## 2026-08-15 (night, round 13) — plugins know where the bar is

Asked whether VSTs need "wrapping" to follow the DAW's tempo. They do not — a host fills in the
transport every block and the plugin does the rest — but checking ours turned up the other half of
the job, and it was missing.

- **Tempo was being sent (fixed earlier tonight); POSITION was not.** `projectTimeSamples` /
  `sample_pos` started at zero when a plugin was *created* and counted up for ever: never reset on
  Play, never told where the song is. So a synced delay was fine (its timing is relative to the
  audio fed to it) but anything with a pattern of its own — Gross Beat, trance gates, synced LFOs,
  arpeggiators — ran at the right speed with an arbitrary phase that changed on every reload.
- **The engine sends an ANCHOR, not a position**: `TransportMsg { playing, t0, beat0, bpm }` — at
  context time `t0` the song was at beat `beat0`. The render thread computes each block's exact
  musical position from it, so there is one message per transport change instead of per block, and
  no drift. Sent on Play, at a song seam (where the bar count CONTINUES — the song moved on, it
  did not restart), on Stop, and to any plugin attached mid-session.
- **`kPlaying` / `TRANSPORT_PLAYING` is honest now.** It was permanently on, so a plugin that
  restarts its pattern on play never saw an edge to restart on. `barPositionMusic` /
  `bar_start_pos` are sent too (VOLT is 4/4), which is what a gate actually locks to.
- The old per-block free-run is kept for plugins nobody is driving — the offline probes, which
  have no song to follow. Three tests: the flags and bar arithmetic a plugin is handed, and that
  the anchor puts every block on the right beat.

## 2026-08-15 (night, round 12) — `.volt` packs the samples

- **Export writes a package now**: a zip holding `song.json`, `samples/NN_name.ext` and a one-line
  README. Send it and the other person has your song exactly as you hear it — no missing samples,
  nothing to relink. Ctrl+Shift+S saves one too (JSON is still offered in the picker, and in the
  palette, for anything that wants a song as text).
- **Written without a library**, per the no-dependencies rule: `zipPack`/`zipUnpack` are a local
  header, the bytes, a central directory and an end record, with `CompressionStream('deflate-raw')`
  doing the compression and STORE as the fallback. Base64-in-JSON cost a third more than the audio
  it carried; the package is smaller than the file it replaces (asserted in the suite).
- **The loader never learned a new format.** A package is turned back into the exact object the
  JSON path produced, so `loadSong` is untouched and every old song opens by the same door —
  opening sniffs `PK` and falls back to JSON. Every entry point (import, drag-drop, Open…, the
  Projects panel) now reads BYTES through `songFromFile`, because reading a zip as text destroys
  it. `tests/pack.test.js`: 12 checks, including the sample surviving byte for byte and a
  truncated package being refused rather than half-loaded.
- Still to do: the desktop folder mirror writes JSON with a `.volt` name, so both shapes live in
  the songs folder. Harmless (the reader sniffs), but it should become a package too — it needs a
  bytes-writing command next to `lib_write`.

## 2026-08-15 (night, round 11) — an installer to hand to other people

- **`voltDesktopBeta_0.2.0_x64-setup.exe`**, 3.1 MB, in
  `native/desktop/target/release/bundle/nsis/`. Built with `cargo tauri build --features asio`
  (NOT `volt_build desktop`, which stops at the exe) after `node tools/mkdesktop.js`.
  Round-tripped before being called done: silent install to a temp dir (`/S /D=…`) → the exe and
  its version resource are right → Start Menu shortcut created → silent uninstall → directory and
  shortcut both gone. Per-user install, so it needs no admin.
- **The installer art is generated, not drawn** — `tools/mkinstaller-art.ps1` renders
  `header.bmp` (150×57) and `sidebar.bmp` (164×314) from VOLT's own palette with System.Drawing:
  the icon's bolt, a faint tracker grid with lit notes, the wordmark, and the teal→magenta rule
  the dialogs use. NSIS wants 24-bit BMPs at exactly those sizes — a PNG there is a blank panel,
  which is why this is a script and not an export. Positions are MEASURED (`MeasureString`); the
  first pass guessed and the two halves of the wordmark overlapped.
- **The wordmark is `volt` + `Desktop` + a magenta `BETA` chip**, in the app, the window title,
  the installer and the Start Menu — nobody should be unsure which build they are running.
- Not signed, so Windows SmartScreen will warn on first run ("More info" → "Run anyway"). Signing
  needs a certificate; worth it only if this goes further than friends.

## 2026-08-15 (night, round 10) — the plugin list is finally honest

- **926 plugins, down from 2179.** The dedupe landed: shell plugins were listed two and three
  times (Waves installs WaveShell at two paths, and the shell offers some plugins at two adjacent
  indexes). RBass now appears once, at the index that actually resolves — verified in the new
  cache, `vst-scan-6.json`.
- **Three plugins that had been invisible for months came back**, all VST 2.3-era files exporting
  `main` rather than `VSTPluginMain`, i.e. the same fix as Sausage Fattener: `Endless Smile`
  (Dada Life), `IL Wave Candy`, and `FL Studio VSTi`.
- **`FL Studio VSTi` is then excluded on purpose** (`SCAN_SKIP` in vsthost) — Mickey: *"fl vsti
  might do issues pleases exclude it"*, and he is right: it is an entire DAW wrapped as a plugin,
  with its own audio device, windows and plugin host, which a scan would open just to read a
  name. Skipped before the file is touched, and refused at LOAD too so a song that already names
  it cannot boot one. One name covers both `FL Studio VSTi.dll` and `… (Multi).dll`; unit-tested
  so it can never quietly widen to the other Image-Line plugins. Cache bumped to `-7`.
- Answered while looking: **Fruity Reeverb cannot be hosted by anything but FL** — its DLL
  exports `CreatePlugInstance`, Image-Line's own plugin API, not VST2 or VST3. The reverbs he
  actually has are Eventide (Blackhole, 2016 Stereo Room), Waves (H-Reverb, Abbey Road Plates and
  Chambers), kHs Reverb, and VOLT's own send/insert.

## 2026-08-15 (night, round 9) — Alt+] from inside a plugin, RBass, and the patch-eating unload

- **Alt+] / Alt+[ now work INSIDE a plugin's window.** Once a plugin has the keyboard nothing
  typed into it reaches the page, so walking the chain meant closing the window first and pressing
  the key again. Intercepted in `pump_messages` like Esc and Ctrl+Delete, and the walk carries on
  from THAT plugin rather than from wherever the cursor is. Watch for `WM_SYSKEYDOWN`: a keystroke
  with Alt held is a SYSTEM key, so matching only `WM_KEYDOWN` would have done nothing at all.
  Cycling also hides the window it leaves, so it flips rather than piles up.
- **"RBass Stereo: no class 864" — a shell plugin renumbered itself.** WaveShell holds the Waves
  catalogue in one file, addressed by class index, and the same file enumerated 861 classes and
  then 609. The name travels with every load now and resolves it when the index does not, and the
  scan dedupes by (name, vendor, format) — his cache had RBass Stereo THREE times, at 864, 865 and
  432, because Waves installs the shell at two paths and the shell offers some plugins twice.
  Cache bumped to `vst-scan-6.json`; `openone` prints class indexes now.
- **Moving a plugin up or down reset every plugin, and it was `vst_unload` doing it.** The unload
  command dropped the slot's patch — reasonable if unload only meant "the user removed this", but
  it is also how the reconciler re-keys a chain, so it deleted the very patches that had just been
  moved to follow their plugins. It also silently defeated this morning's remove-a-link fix. The
  patch stays now; only an explicit chain edit retires one.

## 2026-08-15 (night, round 8) — the five ASIO-only gaps, all the same shape

Every one of these was "the browser still owns something the engine is now responsible for".

- **The master meter read NOTHING** while every channel moved (his screenshot, arrow included) —
  which looks exactly like the channels not reaching the master. They were: `peaks` is indexed by
  track and the master has no track number to sit at, so the master strip simply had no source
  while the engine owned the sound. `MeterData.master` carries the mix peak now, and it folds into
  the master meter, the dB readout and the clip LED the same way `nengPeak` folds into the channel
  meters.
- **Volume and Pan automation did nothing** on engine-owned channels: `AUTO_PARAMS.apply` writes
  to `chans[tr]` — the browser's nodes — and nobody was listening to those. Lane values that map
  onto a mixer field are now remembered (`NAUTO`) and laid over the trackset in `nengMix`, so a
  sweep crosses like any other mix change. Cleared at every pattern start, which is the engine's
  half of `applyBaseAt`. Deliberately not written into `song` — a sweep is not an edit.
- **Editing while playing was inaudible until a restart** ("i need to stop playback and play again
  in order to hear the updated pattern"): the engine holds the cells it was handed at Play. New
  `eng_cells` replaces the notes of the pattern already playing — no clock, no re-aim, nothing
  cut — so rows already booked play as they were and everything ahead of the playhead is the edit.
  Debounced 120 ms off `saveSoon`.
  **And then the ARP still wasn't live, because it does not go through saveSoon at all** — it
  rewrites its notes on every slider move as a preview and only saves when the panel closes, which
  is precisely the case he named. `arpApply` calls the hook itself now. The pattern also travels
  STAMPED (`Pattern::pat`): during a song-mode handover the browser has already moved on to the
  next slot while the previous one plays out, so it cannot know what is sounding — the engine
  matches the stamp against what it is actually playing and ignores an edit meant for elsewhere.
  Both pinned in nengine.test.js, including "a live edit must not restart the transport" and a
  guard that fails if the arp's hook is ever removed again.
- **The metronome now clicks IN THE ENGINE** (`met_click`), scheduled against the same row time as
  the notes, straight to the device past the mixer. Two contexts on two devices cannot be aligned
  by arithmetic — every attempt is an estimate of somebody's real output latency — which is why
  this kept being "not aligned" however the delay was computed. The webview's click returns early
  while the engine owns the sound, so there is exactly one.
- **Ctrl+Alt+R randomizes the notes already on a channel** — same rows, same instruments, same
  velocities, new pitches. It never invents a note: a melodic channel draws only from the pitches
  that channel already plays, and a SLICER draws from its own slices so a shuffled break can never
  land on a slice that does not exist ("so we dont have ghost notes"). New suite
  `tests/randomize.test.js`, 14 checks, including that note-OFFs and disabled notes are untouched.

## 2026-08-15 (night, round 7) — presets survive a chain edit, and FX automation from the main window

- **Deleting a link no longer resets the plugins behind it.** Patches are filed by position, so
  removing link 1 slid link 2 into a slot whose saved patch belonged to the plugin just deleted —
  identity check fails, factory preset. Now every chain edit snapshots the live states first
  (`vst_patch_flush {force:true}` — it collects mid-play too, because losing everyone's preset is
  worse than one block's allocation) and then renumbers PATCHES and IDENTS together
  (`fxvst_patch_removed` / `fxvst_patch_swapped`). The ▲▼ reorder gets the same treatment, and a
  swap is the case a naive rename gets wrong — every entry comes out of the map before any goes
  back. Two unit tests in plugins.rs.
- **VST effect automation is on the main window's right-click now**: the track header in the grid
  and the mixer strip both offer "⚡ <plugin> (automate…)" for that channel's plugin effects, and
  name the knob you last touched inside the plugin when there is one. It leads to the same menu
  the FX link opens — one menu, three ways in. (The instrument version already existed; what he
  asked for was the EFFECT version, where he actually works.)
- The engine now logs `tempo: <bpm> bpm -> N plugin(s)` whenever the tempo it sends changes, so
  "are the plugins on the song's clock?" is answerable from volt.log instead of by ear.

## 2026-08-15 (night, round 6) — the seam, and plugins that know what tempo it is

- **Pattern changes in song mode are SEAMLESS now, because the engine stopped aiming them.**
  A song step used to be applied on arrival: the browser computed `start_in`, and the engine
  applied it against `ctx.current_time()` whenever the message reached it — one IPC hop and up to
  a tick later, differently every time. On top of that, switching `cells` on arrival threw away
  the outgoing pattern's rows between the booking horizon and the boundary, so each pattern lost
  its tail. Both are gone: a turnover is **parked** (`queued`), the outgoing pattern plays every
  row it owns, and the swap happens at `tail_t` — the end of its last row on the clock's own
  drift-free timeline. No wall clock, no IPC, nothing cut: tails and delay repeats ring across
  the join. `from_row` stays 0 (the walk-forward hack existed only to paper over the lateness);
  `start_in` survives as the fallback for a send that lands after the engine has stopped.
  Pinned by `a_pattern_change_lands_exactly_one_row_after_the_last` (clock.rs).
  *"the transition between patterns is not seamless and not in time … there's some delay and its
  preliminary"*.
- **Plugins are told the song's tempo.** Both formats carry it in the transport they were already
  being handed — VST3 `ProcessContext.tempo`, VST2 `audioMasterGetTime` — and VOLT filled in a
  hardcoded **120** for the entire life of the host. So a tempo-synced delay (IL Delay) at 1/8
  computed 1/8 of 120 bpm under a 174 bpm song and repeated off the grid. Now: sent on Play, to
  any plugin added mid-session, at a song-mode seam, and on a BPM change while playing (which
  also re-aims the engine's own clock — it kept the old tempo before). `Plugin::set_tempo` →
  `TempoMsg` → the render thread. Tested in `a_plugin_is_told_the_songs_tempo_not_a_default`,
  which also pins that the musical position (ppq) follows the same number.
- **"Can VOLT automate the knob I just tweaked in a plugin?"** — it already can, and has since
  2026-08-12; it was simply undocumented in F1. Turn the knob in the plugin's own window, then
  right-click the plugin in VOLT (instrument row, or the VST FX link in the Track panel): the
  menu leads with *⚡ Automate "that knob" (just tweaked)*. Now written down in the manual, along
  with the tempo behaviour above.

## 2026-08-15 (night, round 5) — "is channel 4 going through the master?"

Investigated properly rather than guessed at, and the answer has two halves.

- **The engine's routing is correct, and now PROVEN by rendering.**
  `every_channel_goes_through_the_master_and_its_plugins` (native/engine/src/mixer.rs) injects a
  signal into each channel in turn, with one *other* channel carrying a plugin chain (the shape of
  a real session), and asserts that a muting node in the MASTER's plugin chain silences it —
  every channel, every time. It also pushes a mix snapshot AFTER splicing the plugins, because
  `nengMix` fires on every fader move: if `apply` ever dropped the splice, the master's plugins
  would stop processing the moment anything was touched. They do not.
- **The real gap is that there are TWO mixers, and nothing said so.** While the engine is open, a
  channel sounds from the engine OR from the webview (`nengWantsTrack`), and only the engine's
  channels pass through the engine's master — where every VST effect lives. So a webview-owned
  channel genuinely cannot be touched by a plugin on the master, which is invisible and reads as
  a bug. Now: **Ctrl+Space → "Signal path"** lists every channel with notes, says which side it
  is on, and names the fix (ASIO-only, and bounce anything unported with Alt+R); and assigning a
  plugin to the MASTER warns immediately if any channel would go round it.
- Checked while diagnosing: ASIO-only and follow-transport were never overridden in his profile,
  so both are at their defaults (ON) — i.e. every ported channel already goes through the master.
  Also confirmed from the log that the Sausage Fattener fix works in his session: it loads and
  runs, on channel 10 and on the master.

## 2026-08-15 (night, round 4) — six things Mickey found in one message

- **The grid drop never wrote its note, and the reason is worth keeping.** An OS drag is
  synthesised by the webview, not by the page: the dragovers tracked the pointer correctly (the
  cell lit up), and then the `drop` arrived at the DOCUMENT instead of the grid. The grid's
  handler never ran, so the sample became an instrument and no note was written. Both handlers
  place notes now, and neither trusts `e.target`: `dropCellAt` takes the pointer's hit-test
  first, then the cell the last dragover LIT UP (evidence of intent when the webview gives us
  nothing), and the highlight is dropped the moment the pointer leaves the grid so it can never
  hijack a drop elsewhere. New suite `tests/drop.test.js` — 13 checks, including the
  document-delivery case that was the actual bug.
  **Dragging a hit out of the Sample Search panel onto the grid now lands a note too** (it did
  nothing there before), reusing the instrument when that file is already in the song.
- **The metronome was misaligned, twice over.** (1) `metOut` fed `spkOut`, so "ASIO only" — the
  desktop default — muted the click along with the webview's mix. It goes straight to the output
  now; it taps no meter and renders never schedule it. (2) Its delay exists to match the
  browser's mix, which is late by two compressors plus the master chain. The engine's audio does
  not take that path — it is aimed at the browser's SCHEDULING time — so the same delay pushed
  the click 12 ms late bare, 40–60 ms with a Limiter. `metLatApply()`: the click lines up with
  whoever is making the sound, on nengSpk's condition, never re-derived.
- **VST FX links reorder** — ▲▼ on each link, automation lanes moving with them; the engine
  re-splices from `(track,pos)` as it already does. `fxVstRemove`/`instVstRemove` are now one
  implementation each, shared by the panel buttons, the hotkey and the plugin window.
- **Ctrl+Delete removes a plugin, and works INSIDE the plugin's own window.** The pump
  intercepts it (as it does Esc), queues the HWND, and the plugin thread matches it to a slot and
  emits `volt:vst-del`; the page decodes instrument-vs-chain from the editor slot and edits the
  song. Rust never touches the song — the song is the page's.
- **Alt+T was never legal and was hiding from the scanner.** Alt+T is Firefox's Tools mnemonic,
  and the check only looked for `e.altKey` and the letter on ONE line — the binding sat inside an
  outer `if(e.altKey)` block, so it passed for months while doing nothing in Firefox. It is
  **Ctrl+Alt+T** now (a chord Firefox does not claim), in the handler with no left-rail bail, so
  it also works with the instrument list focused — which is why it "did nothing" on the desktop.
  Both copies of the guard now allow ctrl-qualified Alt chords and stay strict for bare Alt.
- **"Why don't I see Sausage Fattener?" was TWO bugs stacked, and both are fixed.**
  1. `C:\Program Files (x86)\VstPlugins` was excluded because it "is 32-bit by definition" —
     but installers drop both builds in it (`Sausage Fattener.dll` x86 beside
     `Sausage Fattener x64.dll`, and the x64 one is fine). Architecture was always decided per
     FILE from the PE header, so scanning those folders costs nothing and hides nothing.
  2. With the folder scanned it STILL did not appear: `openone.rs` said "not a VST2 (no
     VSTPluginMain)", and its export table has exactly one symbol — `main`, the VST 2.3 entry
     point. `vst-0.4.0` looks up `VSTPluginMain` only; every real host tries both. Hence a
     second vendored crate, `native/vendor/vst`, with one `VOLT FORK` line
     (`VSTPluginMain` → `main` → `main_plugin`), patched in from all three workspace roots.
     Verified: `[Vst2] SAUSAGE FATTENER — DADA LIFE (Fx)`, opened cleanly.
  Scan cache bumped to `vst-scan-5.json` — a `-4` cache is missing both classes of plugin, and a
  cache is trusted on sight. This very likely un-hides other old favourites too.

## 2026-08-15 (night, round 3) — drops on the grid, the search that comes back, FX windows on keys

- **Audio dragged from Explorer onto a GRID CELL drops a note there** — the cell lights up as
  you hover, the drop makes (or finds — same dedupe) the instrument and writes a C-4 at that
  exact track/row; several files stack down the rows. Off the cells (or a project file) it
  falls through to the old behavior. Mirrors the recorder-drag path.
- **The last Everything search survives a restart** (`volt.es.q`) — the panel comes back with
  the query re-run, results and all, a beat after boot and only if the field is untouched.
- **Alt+] / Alt+[ walk the channel's FX plugin windows** — first press reveals link 0 on the
  cursor's channel (master view = master chain), then next/previous with wrap. In the palette
  and F1 too.

## 2026-08-15 (night, round 2) — the slicer-under-FX silence

- **A SLICED instrument's audio never crossed to the engine.** All three sample-upload sites
  filtered `instr==='sampler'`, while `nengPattern` marks sampler AND sliced cells with a
  `sample` id — so the engine refused every slice note ("sliced has no sample loaded") and the
  channel went silent the moment the engine owned it (an FX VST on the channel, or ASIO only).
  Exactly *"slicer on channel 12, i put an fx over the channel, audio is not processing"*. The
  browser played the same notes fine, which made it look like the FX ate them. Uploads now key
  off `c.sample` — the sender's own marking, one source of truth. `trig()`'s early-out also now
  mirrors the sender's sliced refusal (no audio / no markers ⇒ keep sounding in the browser).
  Regression-tested in nengine.test.js: the sliced upload, its cell's sample id, and the flat
  slice params all asserted.

## 2026-08-15 (night) — VST presets survive a restart

- **Plugin states persist per song on disk** — `%LOCALAPPDATA%\VOLT\patches\<song-id>.pat`,
  written on every transport stop, every device close, and awaited on quit before the process
  hard-exits. The in-memory PATCHES cache (reopen survival) now has a disk twin (restart
  survival); `vst_load`'s patch-at-create path is unchanged, it just finds presets waiting.
  Per-song files + replace-not-merge on song switch, so presets never bleed between songs.
  A crash costs at most the tweaks since the last stop. Details in CLAUDE.md.
- Not done: pruning `.pat` files of deleted songs (harmless orphans, a few MB each at worst).
- Vault/phrases checked: `tests/vault.test.js` — 12 checks green.
- Desktop rebuilt + relaunched with all of today's UI work (Sample Search above Instruments,
  zoom, drops, spinners, middle-drag pan).

## 2026-08-15 (evening, round 3) — zoom ×4, and the mouse does more

- **Zoom ease τ 12 → 3 ms** ("accelerate it x4") — lands in 1-2 frames; just enough glide left
  that it doesn't teleport.
- **Middle-drag pans the grid** — FL's hand tool, no modifier. Capture-phase, because the cell
  mousedown doesn't check the button and would have moved the cursor.
- **Wheel over any toolbar field nudges it** — BPM (Shift = ±10), Oct, Step, Swing, Pattern,
  Len, R/Beat, Trk. Selects walk their options. Everything goes through the field's own
  'change' handler, so it cannot drift from typed input.
- Both new gestures are in the F1 manual.

## 2026-08-15 (evening, round 2) — zoom at 5 frames, samples flow in from anywhere

- **Zoom ease τ 28 → 12 ms** — lands in 4-5 frames at 60 fps, per Mickey's spec ("up to 5 frames").
- **Sample Search moved above Instruments** in the left rail — a hit drags one short hop onto a
  row now. Fold state is class-keyed, so the move was free.
- **Dropping a search hit on an instrument row asks Replace / Add new** (it used to replace
  silently — every other drop-on-a-row already asked).
- **Audio dragged from Explorer / other apps onto the window** already became an instrument;
  now it DEDUPES: identical bytes (b64 string compare) select the existing instrument instead
  of adding a copy. Same dedupe on the instrument-panel drop, with an honest count in the status.

## 2026-08-15 (evening) — the zoom got fast, and the first motion pass

- **The zoom glide was frame-based, and that was the slowness.** Every zoom frame re-lays the
  grid out, so on a busy grid the frame rate drops — and the old per-frame easing constant
  (`k=0.28` per rAF tick) stretched the animation exactly when frames got slow. The ease is
  TIME-based now (τ = 28 ms in log space): the move lands in ~100 ms at any fps, and a slow
  frame takes a proportionally bigger step instead of adding time.
- **Motion polish, first pass** — transform/opacity only (compositor, never layout), all behind
  `prefers-reduced-motion`: dialogs and the palette pop in with a small overshoot bounce, menu
  dropdowns and the Vault/settings panels drop in, buttons compress on press, and a new status
  message rises into the footer instead of teleporting.
- **A tip on boot**: ~2 s after load, if the status bar is idle, one random entry from the F1
  manual shows for a few seconds. The manual is full of tricks nobody presses F1 for.
- Candidates for the next pass (not done): a subtle flash on the pattern chip when it changes,
  a pop on newly added instrument rows / song slots, drag ghosts with tilt, a short shake on a
  refused action (e.g. blacklisted plugin), playhead-row glow in the grid. All must stay
  transform/opacity-cheap — the grid runs at 60 fps while playing.

## 2026-08-15 (later) — everything else that was broken, and the UI work

With the distortion dead the rest of the day was Mickey using VOLT and finding what it does
badly. All of it is fixed and tested:

- **Song mode chopped every pattern's tail.** The browser sends the next pattern up to a
  lookahead EARLY; the engine treated that like a fresh Play and cut every voice and note-off'd
  every plugin the moment it arrived. Now a mid-play arrival is a HANDOVER — the new pattern is
  armed, the ringing audio is left alone — and only the user's own Play does the hard reset.
  *"it goes to the next pattern too early and too slow getting back to the 1st."*
- **The top menu barely worked.** Every item is a child of its menu title, so an item's
  `mousedown` bubbled up, closed the dropdown, and the `click` landed on nothing. One guard.
- **Drag-and-drop was dead in the desktop build** — `dragDropEnabled: true` lets WebView2 install
  its own drop target and the page's HTML5 drag-and-drop stops existing. That killed both slice
  drags AND dropping a WAV or a project in from Explorer. Off now; the page owns drops again.
- **Ctrl+O** opens a project. **Ctrl+X over a bare note-OFF** removes it instead of putting the
  terminator straight back.
- **A grid zoom, FL's gestures**: Ctrl+wheel = rows, Alt+wheel = columns, Ctrl+Alt = both,
  Ctrl+0 resets, level remembered. It glides (eased in log space, one paint per frame, anchored
  on the row under the pointer for the whole animation) and the channel headers stay pinned to
  their columns to 0 px at every level — that last part by re-running `syncHeadWidths()`, the
  mechanism VOLT already had, rather than the CSS I started rewriting before I found it.

**And the lesson that wasted the most time:** `cargo build` then `mkdesktop` ships an exe wrapped
around the PREVIOUS page, and the documented sha256 check passes anyway because staging is what
makes the two files match. Two rounds of "the fix does nothing" were that. Stage first, and
verify the honest way — the exe's mtime must be newer than dist/index.html. Recorded in
CLAUDE.md.

## 2026-08-15 — THE DISTORTION, actually dead: one sign flip in the vendored crate

Two days, five real-but-wrong fixes, and the ending is one line. The crate evaluated a future
`setTargetAtTime` before its start time — the exponent flips sign, `exp(+(T0−t)/τ)`, and a cut
scheduled 160 ms ahead with τ=2.5 ms saturates the gain to f32::MAX instantly. Every retriggered
native note played at full scale until its cut landed; the limiter squared it off at note pitch.
Mickey's observations were the map all along: *distorts with a VST on the channel* (those are the
channels whose native voices play in the engine), *clean without* (browser plays those, no bug),
*clean on Sytrus* (plugin notes, no native voice), *worse with density* (more retriggers), *every
device* (the poison is upstream of all of them). The tap (`VOLT_TAP=1` → tap.wav) ended it in one
look: ±0.92 square at the note's pitch = huge signal + limiter, not clipping, not dropouts.
Fixed with two VOLT FORK guards in param.rs; regression test asserts peak sanity over the WHOLE
render, because the old cut test measured only the after-region — precisely where the bug wasn't.
Worth reporting upstream. And the day's wrong-but-real fixes stay: one mix instead of two, cut
voices reclaimed in 120 ms, the delay-time detonator guarded, fixed-format inserts, the
continuous-processing silence flag, the tap itself.

## 2026-08-14 — THE DISTORTION: two mixes were running, and one of them was invisible

Mickey, third report, sharper than any measurement: *"stop the legacy plugins distorting on notes
play"* — and his instinct about the cause was right even though the migration he suspected was
already finished. All fourteen instruments have been native since the previous night. What had
NOT stopped was the **webview synthesising them anyway**.

`trig()`'s early-out — the branch that makes a voice silent because the engine is playing it —
tested `song.trackset[…].fxVsts.length`. That was correct when a VST effect was the only way the
engine could own a channel. ASIO-only then started sweeping EVERY channel to the engine, and the
copy in `trig()` never learned: on any channel without an FX plugin the browser built the full
voice — oscillators, filters, shapers — for notes the engine was already playing. `spkOut` muted
the result, so it was perfectly inaudible and cost exactly as much CPU as if it weren't. Two
complete mixes, one machine, and on busy bars the ASIO callback starved. Distortion that arrives
**with the notes** and worsens with note density: the snare-plus-reese bars first.

The fix is one call to `nengWantsTrack` instead of a hand-copy of half of it, and the rule it
teaches is in CLAUDE.md now: ownership is decided in exactly one function, never re-derived. The
test counts `createOscillator` calls, because a gain-value assertion passes in both worlds.

## 2026-08-14 morning, part two — the distortion hunt: what it is NOT, and the two real finds

*"the snare fucks it up with distortions like crazy"* on the all-native path. Hunted with
measurements, and the suspect list is now short because most of it is ACQUITTED with numbers:
the strip knobs are benign (his song: drv 0, eq flat, cmp 0 — read from dnb8.volt directly);
the master graphs are identical architectures; and the new `probe-comp-slam` job nulls the
safety limiter at his exact settings under constant deep GR at **-24.5 dB rms, level +0.49 dB**
— the two limiters track. The DSP is not the distortion.

What remains, and what changed:
- **The buffer default was the driver's MINIMUM** ("Interactive"). A clean 24-voice probe proves
  nothing about a full mix with five plugin instances — device-level starvation does not blow
  the render budget, so the underrun counter stays quiet while the driver glitches. Never-chosen
  now defaults to **23.2 ms** on desktop (FL's reasoning exactly); an explicit "Automatic" in ⚙
  is honoured. If the crackle survives this, the next suspect is ASIO4ALL itself → UMC native.
- **A close-path DEADLOCK is real and caught**: the 08:11 instance never logged "window closed"
  (force-killed), and pid 16120 froze on CloseMainWindow — zero CPU, not responding, the freeze
  signature of a lock. Prime suspects: OPEN_GATE held across a slow driver open while the close
  path wants the engine, or the exit patch-capture. Top of the next session (task list #9).
- Two app instances overlapped this morning (08:13 started while 08:11 was frozen-alive) — the
  single-instance question is now worth asking.

## 2026-08-14 morning — ASIO-only defaults ON, because the toggle was the last lie

The morning after the ports, Mickey heard distortion on his reese channel and concluded *"all
the synths are still the browser shit"* — and with ASIO-only OFF he was right: the ownership
rules kept plain channels in the webview, and the one switch that changes that was a palette
toggle he had no reason to know about. A capability that needs a secret handshake is not a
capability. So on DESKTOP, `NENG.only` (and follow-the-transport with it) now defaults ON when
never explicitly chosen — an explicit '0' from the toggle is honoured forever, the browser build
is untouched, and a failed device open still sounds through the webview exactly as before.

Two more findings from the same morning's log: the patch-carry and clean-close machinery worked
in production on their first real outing (`patch kept: i7 (1845 bytes)` — the Sytrus preset —
plus six FX patches, then `device closed and released`), and the distortion suspect list for
channel 6 narrowed to **RBass on the native master** — the one processor that ONLY
engine-owned channels pass through, which is exactly the shape of "distortion only on the reese
channel". Removed via the assist channel for an A/B; undo restores it.

## 2026-08-14 — every instrument is native: the one-engine decision, and the ports in one day

Mickey called it: *"lets do whatever needed to make it work in the same audio engine — no
seperation bs."* The plan is staged (see P3), and stage one landed the same day: **all fourteen
instruments now exist in the Rust engine**, each one a transliteration of its `trig()` branch,
each one proven by rendering the same note through both engines and subtracting.

The day's numbers, so nobody re-argues them by ear: psybass **-53.7 dB** rms (the shaper
flattens the saw-table gap — better than the raw saws it is made of), glitch FM mode **-74.3**,
acid303 -44.6, sine-zap glitch -47.7, noise glitch -53.1 *with chaos on* (the seeded scatter is
bit-exact: JS's ToInt32 seed fold and the rnd() call order ported precisely), mutant -27.1,
supersaw -27.2, pluck -28.2 — the last three sitting on the known WAVEFORMS octave-table gap,
levels matching to a third of a dB. One measurement note for the future: glitch's oscillator
modes null only with grains on quantum boundaries, because Chrome quantises mid-quantum
frequency changes to 128-sample blocks — jittered grains match in level and decorrelate in
phase, and no port however correct can null that.

What this buys today, before any flip: **"ASIO only" in pattern mode is the complete one-engine
VOLT** — every channel through the chosen device, no unported holes, one clock. The flip itself
(song mode in the engine's transport, then deleting the split) is specced in P3 and deliberately
not rushed into the same night.

## 2026-08-13 night — the panic could not win, because it was fighting a ghost

*"THE PANIC not killing the notes."* Alt+P is a careful ladder — stop, cut every webview voice,
`eng_panic` into the engine, drop the device if it does not answer — and every rung worked. It was
addressing the wrong engine.

`AudioContext`'s `Drop` in the vendored crate **leaks the running backend on purpose** so a script
that lets its context go out of scope keeps playing. VOLT drops a context every time it reopens a
device, which this session made easy to do: the buffer knob, the picker, the self-heal. Each one
left an orphan still rendering the old graph — its plugins, its held notes — with no channel, no
handle and no command that could ever reach it again. Panic silenced the live engine; the ghost
played on. `eng_close` did not help: it frees the handle, which is exactly the thing that was
already gone.

The log had been shouting it for an hour in a language nobody reads: **36,659 lines of "underrun
#1"**, ten a second. One counter cannot produce that. Several capacity monitors can — each
belonging to a context that was supposed to be dead, each seeing another's number as "before".
Four contexts opened between 20:34 and 20:35; four engines were running.

So the audio thread now ENDS properly: cut voices → all-notes-off into every plugin →
`ctx.close_sync()` → tell whoever is waiting. `eng_open` waits (bounded, two seconds, because a
wedged thread must not freeze a device change) for the previous engine's word before it touches
the hardware. Monitors carry a generation and only the current one speaks.

And the same reopen explains **"i put a preset in Sytrus and after some time it reset"**: a reopen
replaces every plugin with a brand-new instance at its factory preset. Patches are now taken off
the old plugins as the engine closes — `VstNode::state` → `StateMsg`, answered on the render thread
where the plugin actually lives — kept by owner AND plugin identity, and pushed back into the new
instance before it plays a note. In memory, so far: survives a reopen, not a restart. The song half
is P2 and wants the sample store's design, because a Kontakt patch is megabytes and the autosave
must never be what breaks.

Two smaller things asked for in the same breath: **Alt+Enter** opens the selected instrument's
plugin window (Windows' properties chord — one of the few left, and it lives in the global handler
because you press it straight after clicking an instrument, which leaves focus in the rail), and
**Ctrl+Space now finds generators, not just effects**: type "diva" and the first row is *replace
instrument 03 with Diva* — asked for by name, undoable, with *new instrument* right behind it.

## 2026-08-13 night — the vanish was UNDO, and the buffer knob exists

*"the vst inst i put are vanishing"* — and the log showed the shape: after a device reopen the FX
chains reloaded and the instrument plugin did not, because the SONG no longer claimed it. What
stopped claiming it: **the undo snapshot**. `snapState()` listed an instrument as
`{name,type,params,sampler bits}` — no `ins.vst`, no `masterFxVsts` — so one Ctrl+Z after adding
a synth handed `applyState` a song without the plugin, and the autosave scheduled 700 ms later
wrote that loss to disk. A tracker player's hands hit Ctrl+Z constantly; the plugin was doomed
within minutes of being added, every time, and it looked like the engine ate it.

Four fixes, one rule — **the song is the only truth, everywhere**: the snapshot carries `vst` and
the master chain now; `applyState` restores both and then converges the ENGINE to the restored
song; `nengVstSync` reconciles instead of only loading (a slot the song stopped claiming is
UNLOADED — so delete, replace, clean-unused and undo all converge, and a stale engine plugin can
no longer double a channel); and `nengPickVst` writes the song BEFORE the 0.2–1.5 s load, the
same lesson `nengAssignFxVst` already had carved into it. Covered in `nengine.test.js`:
plugin-through-undo, engine-follows-removal, and the buffer row below.

And the FL screenshots' point — **the buffer length knob** — is in: device picker ⚙ row,
Automatic or 1.5/2.9/5.8/11.6/23.2/46.4 ms (the 64…2048-sample ladder, said in ms so it means the
same at any rate), persisted, sent as the open hint, applied by reopening on the spot. With it,
the counter that tells you when to reach for it: the engine's render capacity monitor parks
avg/peak load + underrun events in a static, `eng_stats` serves it, the badge shows `N⚠`, the
status bar names the cure once per half-minute at most, and every underrun is one log line with
a timestamp — so "it crackled at 20:14" is now answerable from `volt.log`.

## 2026-08-13 late night — the LAST distortion: allocations in the audio callback

The clincher came from Mickey's own A/B: an UNTOUCHED apQualizr2 distorting a frozen (bit-clean)
bounce, "only where the note is". Every layer measured clean offline — plugin (fxprobe), worklet
wrapper (new `wkprobe`: −∞ dB residue, stereo, 0.9 peak), stereo sampler path (new rendered
test), the reese itself (−47.6 dB null) — so the fault had to be LIVE-only: the multi-bus
`process_io` was allocating half a dozen Vecs per 128-sample block on the render thread, and an
allocator stall in the callback is an underrun — audible exactly and only where the music is,
on sparse material. `process_io` now runs on scratch allocated once at first block: **zero
allocations in the callback after warmup.** Offline probes re-run clean; if any distortion
remains live it is the plugin's own knobs, provable by the same probes.

## 2026-08-13 night — WHY the reese went berserk: the engine dropped the tracker commands

*"explain me why the reese going berzerk when putting vst over"* — because putting a VST on a
channel hands that channel to the engine, and the engine ignored Gxx/Cxx/Uxx/Dxx. His reese
line is 10 notes carrying 3 glides, 7 filter cuts and a slide: without them it machine-gun
retriggers with the filter stuck open. Counted straight out of dnb.volt — the line WAS the
commands.

Ported: `Voice` grew the browser's bend (one ConstantSource into every pitched source's detune —
sub, reese, sampler) and the Cxx filter handle; `glide_to`/`slide`/`set_filter_target` are
transliterations of baseVoice's; cells cross with `cmd`/`cmdv` and the engine's scheduler plays
them by schedRow's exact rules (glide only a live voice, C on whatever the lane now holds, U/D
accumulate). Proven by rendered audio: an octave Gxx doubles the zero-crossing rate and the
mid-glide sits BETWEEN the notes. Still browser-only: Rxx retrigger rolls, Pxx per-note pan,
painted `p.*` lanes — noted, next.

## 2026-08-13 night — the disappearing-VST bug, caught by bisection, dead

*"the vsts getting died after i put them… tried to add over channel 5 and nothing appears."*
Reproduced headlessly, then bisected with three probes in one test run: the reconciler was fine,
the plugin identity was fine — **the assign function itself** was the trigger. `normTs()`
REPLACED `ts.fxVsts` with a filtered copy on every mixer render, so any render between "grab the
chain" and "push the plugin" left the code pushing into an orphaned array the song no longer
owned; the reconciler then saw a song without the plugin and dropped it. Fixed twice over:
normTs only reassigns when it actually drops junk, and `nengAssignFxVst` re-reads the song's
array at push time (plus says out loud when a load goes nowhere, instead of silence). The
palette test now renders the mixer mid-flow to keep the trap sprung forever.

Also: Ctrl+Space direct plugin search ("+ L2 Stereo → this channel"), scan hints instead of
empty palettes, the FX picker warns about unported instruments IN the dialog, +fx selects its
channel so the panel follows, and the assist channel (remote hands + `info` op) is live.

## 2026-08-13 evening — the REESE is in the native engine

The hole every FX complaint kept falling into ("apq on the Reese channel hears nothing") is
closed: `reese` is ported to `native/engine` — nv detuned saws → drive shaper → resonant lowpass
with the sine-LFO'd cutoff, optional sub sine, sustained like `sub`, released over its own
`rel`. Proven by rendered audio in a Rust test using Mickey's actual dnb5 Reese preset
(fractional voice count, sub layer, held/released). `NENG_OK` now carries it, so VST effects on
a Reese channel actually process it and ASIO-only mode plays it. Follow-up owed: a
`tests/parity.test.js` job to null it against the browser like the other ports.

Also from this pass: the plugin flush is gated to USER Play only (mid-play re-sends and song
turnover no longer wipe plugin state — that was audible), and VST2s are never flushed from the
render thread (FL plugins punish dispatcher calls off their creation thread). The snare mystery
was mixer state, not code: channel 2's fader at 0 with its FL filter closed.

## 2026-08-13 afternoon — Stop kills queued notes, Play starts CLEAN, +fx on every strip

Three from testing: *"i get the vsts stuck at times… right now i need ctrl+r to stop the
notes"*, *"when i hit play i need the vsts to reset so it will not carry shit from delays and
tails"*, *"i need it to work on each and every channel"*.

1. **Stuck notes: found and fixed.** Notes are scheduled INTO plugins ahead of time (wider than
   ever with PDC), and Stop only sent note-offs for held notes — the future note-ons queued in
   each node fired after their offs and rang until Ctrl+R. Panic always dropped the queue; now
   **Stop does too**.
2. **Play starts from silence.** New `ResetMsg` → `Plugin::flush_state()` (VST3 `setProcessing`
   toggle, VST2 mains toggle — each format's documented flush): queued notes dropped, 128-off
   sweep, and the plugin's OWN state — voices, delay lines, reverb tails — cleared, instruments
   and effects both, on every Play and on panic (a delay still repeating IS the stuck sound).
   Stop deliberately lets tails ring out — that's musical; Play is the fresh start.
3. **`+fx` button on every mixer strip** (F9) — add a VST effect to any channel from the mixer,
   no need to select the channel first. Track panel and palette still work as before.

Known gap, noted: plugin latency is captured at ATTACH; a plugin that changes its own latency
live (linear-phase toggles) won't update PDC until reloaded — handling VST3's
`restartComponent(kLatencyChanged)` is the future fix.

## 2026-08-13 late morning — the nudge is DEAD; alignment is automatic now

Mickey: *"does it look logical to u that ill have to do nudging as part of my workflow? we need
pdc… automatically. btw when i put l2 it was around 35ms nudging"* — and that 35 ms was the
measurement that cracked it: it matched (browser bLat − engine reported) within a couple of ms,
which means on the shared Windows path the engine's REAL latency equals the browser's and the
reported number is the fiction. So `nengLatCorr` now aligns per stack, automatically: ASIO by
its honest numbers, WASAPI/default by the equal-real-latency rule (correct by nothing). The
nudge survives only as a ±few-ms taste trim, on a fresh storage key so the morning's hand-set
35 ms cannot double-apply. Asserted in tests both ways. The true endgame stays the mixer plan:
one engine owning every channel, one clock, no cross-stack anything.

## 2026-08-13 mid-morning — the apQualizr2 silence was OURS, and the last delay gets a knob

*"apq on drums not showing and mutes the channel"* — reproduced offline in one command
(`fxprobe apqualizr`: 0.500 in → 0.000 out) and fixed in the host: **`ProcessData` now presents
every audio bus the plugin declares**, sidechains as honest silence. apQualizr2 declares a
sidechain input, was handed `numInputs:1`, and refused every block — which in a chain mutes the
whole channel. After: 0.500 → 0.500. WaveShell L2s and Vital re-probed, unaffected.

*"still delay on kicks"* even with both engines on one device: driver-reported latency is an
estimate (shared WASAPI's is fiction), so **Ctrl+Space → "Native engine: timing nudge…"** —
step the engine earlier in 10 ms notches while the pattern loops, by ear, takes effect within a
bar, remembered. The same knob every DAW ships as "driver offset".

## 2026-08-13 morning — ASIO4ALL wedged the audio thread; now it can't brick a launch

First test of the chains build looked broken: every plugin "the AUDIO THREAD never answered in
30 s". Stacks, not guesses (cdb): the audio thread was parked FOREVER inside ASIO4ALL →
Windows' `wdmaud` GS-synth teardown — the same sick audio layer as yesterday's zero-endpoints
episode, wedging after the open had already reported success. Not the new code; reproduced on a
clean relaunch.

Shipped the defense: a device journal (`dev-opening.txt`, the vst-loading.txt trick). Written
before every open, erased only when the audio thread proves it is alive by reaching its command
loop; a device still in the journal is refused by the AUTO open and the picker comes up instead —
picking it by hand retries. So a driver having a bad day costs one bad session, not every launch.

**Also shipped, same morning:** *"can we have direct sound so we dont need to bs with asio"* —
the device picker now leads with **Windows default output** (id `DEFAULT` → the crate's empty
sink; never ASIO): zero setup, and it puts BOTH engines on one device, which also collapses the
two-DAC timing skew that read as "still delay when putting l2 on kicks" (the L2 itself is 64
samples and compensated — the offset was the kick leaving a different sound card than the rest
of the song). And `nengFxTrapScan` says at LOAD time which instruments a channel's VST effects
cannot hear (the add-time warning never fired for chains already in the song — the Reese/
apQualizr2 confusion, twice).

**Resolved 09:40:** after killing every wedged VOLT instance and relaunching, the SAME WASAPI
device opened cleanly — context → mixer → "alive and pumping" in under a second, every plugin
attached, chains live, L2's 64-sample lookahead compensated. So the wedge was the machine's
audio layer left poisoned by the first stuck open (each later open, ANY driver, queued behind it
kernel-side); killing all stuck processes released it. Breadcrumb logs now mark each startup
stage in volt.log, so if it ever wedges again the log says exactly where. A Windows reboot is
still the real cure for the layer itself, as on 2026-08-12.

## WHAT LANDED 2026-08-12 (late night) — FX CHAINS, latency compensation, add-from-palette

Mickey: *"u hear clearly that there is latency. is it pdc things? lets fix!"*, *"why cant i add
multiple vsts over mixer channels?"*, *"lets simplify... from the search ^space"*. All three:

1. **PDC — plugin latency is compensated now.** L2's lookahead is thousands of samples and the
   drums really were that late. Every plugin reports its latency; the engine schedules its notes
   EARLY by the channel chain's total plus the master's, so the wet audio lands ON the row.
   Nothing to configure. (The very first row after Play can't be reached early — it lands late
   once, then everything locks.)
2. **FX chains.** Channels and the master take MULTIPLE VST effects now, in series, top to
   bottom — `+ VST` in the Track panel appends, ✕ removes one link, right-click a link to
   automate that plugin. Old one-slot songs load as a one-link chain. Removing a middle link
   reloads the ones behind it (plugin state isn't saved in the song yet, so nothing extra is
   lost).
3. **Ctrl+Space → "Add a VST effect to this channel…"** (and "…to the MASTER…") — type to filter,
   Enter, done. Same picker as the panel button.
4. **The two honest no-signal cases now say so out loud** when you add the effect: a channel
   whose instrument isn't ported yet (Reese — the engine has nothing to feed the plugin, the
   channel keeps playing dry), and a master effect in "+web" mode (it only hears the engine's
   share until you switch to ASIO only). This is what "apqualizr2 not working" and "L2 on master,
   no signal" actually were — apQualizr2 itself loads and runs fine.

870 JS + 39 Rust checks green (new: chain wiring proven by audio in series, reconciler
add/remove/migrate, legacy one-slot songs).

## WHAT LANDED 2026-08-12 (night) — mute works everywhere, FX VSTs work on EVERY channel

Mickey: *"when muting a channel that has a vst on it its not muting"* and *"I cant put fx vst over
native tools channels — the sound is muted and theres no signal going through the vst"*. Both
real, both fixed, and the dig turned up something bigger:

1. **The engine had never played a single native instrument from the app.** `nengPattern` read
   `ins.p` — a field that does not exist (`.params` is the real one) — so every kick/snare/hat/
   sub/sampler cell crossed with no parameters and Rust refused it. Nobody heard it because the
   webview played the same notes anyway… until a channel FX VST silenced the webview's copy and
   the channel went DEAD: silence, empty plugin meters. One word fixed it (`ins.p`→`ins.params`);
   the test that should have caught it was hand-building instruments with `.p` and now builds
   them the way the app does.
2. **Mute and solo reach the native mixer now.** The browser mutes by not scheduling notes — which
   does nothing to a channel whose sound lives in the engine (every plugin channel). The mute
   button now crosses in `eng_mix` (folded through `trackAudible`, so solo and poly lanes behave)
   and gates the channel's fader in Rust — instant, both directions, mid-note. Proven by audio in
   a Rust test: muted strip renders silence, solo silences the others.
3. **Every channel has ONE owner now** (`nengWantsTrack`): plugin + FX-VST channels sound from the
   engine, the rest from the webview, "ASIO only" (pattern mode) hands the engine everything
   ported — at 0..64 velocity as written (it was /127: **every plugin note ever played was at
   half velocity**). Changing ownership mid-play (add/remove an FX VST, toggle ASIO-only/follow)
   re-sends the pattern immediately instead of waiting for the next Play.
4. **A song whose only plugins are channel FX loads them now** — the warm-up and the sync before
   Play both skipped `nengFxSync` when no plugin *instrument* existed.
5. Badge/status/dialog texts rewritten to say the ownership rule in plain words, and "ASIO only"
   names the instruments it cannot play instead of letting a channel die quietly.

864 JS + 37 Rust checks green.

## 2026-08-12 (evening) — "the plugins not opening": diagnosed to WINDOWS, not VOLT

Mickey: *"is it me or the plugins not opening?"* Neither — measured down four layers: VOLT's log
(no `vst_load` since 11:30, every window "REFUSED") → `volt_devices` (ASIO only, zero WASAPI) →
a scratch cpal project (zero devices) → a raw COM `EnumAudioEndpoints` probe from PowerShell
(**zero render endpoints in ANY state, machine-wide**, MMDevices registry empty, all drivers
healthy). Windows' audio endpoint database is wedged; service restarts did not rebuild it; a
reboot is the fix. VOLT's chain of dominoes: remembered device gone → engine never opens →
plugins never load → windows refused. **The UMC's own ASIO driver bypasses the broken layer and
was verified playing (40 ms)** — the session was usable the whole time through it.

Two hardenings shipped from it: a missing remembered device now auto-opens the device picker
(one status line was how this reported itself, for six hours); and opening a plugin window with
no engine open says the cause and offers the picker instead of "REFUSED: no plugin on this
instrument". Full trap notes in CLAUDE.md. `examples/devdump.rs` (engine crate) stays as the
enumeration diagnostic.

## WHAT LANDED 2026-08-12 (fourth pass) — automation by RIGHT-CLICK, with the last-touched knob

Mickey: *"as for the automation why not by right click like in the native tools?"* Right. Landed,
852 JS + 53 Rust checks green, exe shipped:

- **The host listens for knob touches now.** Plugins report every knob turned in their own editor
  — VST3 through `IComponentHandler::performEdit` (a real handler in `host.rs`, set on every
  controller; `EditHooks` lives as long as the plugin), VST2 through the `audioMasterAutomate`
  callback. Each instance carries a `tag`; one host-wide touch log (`last_touch()`), a
  tag → slot/track registry on the desktop side, and a `vst_touched` command that answers
  "which plugin, which parameter, what name, how long ago".
  `performEdit` is deliberately NOT forwarded to the processor — every plugin here already routes
  its own GUI edits to its own audio (that is how they all worked with a null handler).
- **Right-click a plugin in VOLT = the automate menu**, same gesture as VOLT's own sliders:
  the instrument row of a plugin instrument, or the VST FX slot in the Track/master panel. The
  menu leads with **⚡ Automate "«the knob you just tweaked»"** when a touch is fresh (two-minute
  window — tweak, audition, come back), then the full parameter picker, then Open window.
  The JS polls `vst_touched` at ~1.4 Hz and hints in the status bar (throttled) when a new knob
  is touched, so the gesture is discoverable.
- Not done (and known): right-clicking **inside the plugin's own GUI** pops the plugin's menu,
  not VOLT's — that needs `IComponentHandler3`/`IContextMenu` (VST3-only, plugin-dependent), a
  separate COM object worth its own session. The tweak-then-right-click-in-VOLT flow is the same
  workflow FL's "last tweaked" serves.

## WHAT LANDED 2026-08-12 (third pass) — master VST FX, dedupe, badges, and PLUGIN AUTOMATION

Mickey's third list of the day. All landed, 847 JS + 53 Rust checks green, exe shipped — and the
launch log showed him already using the previous pass: an L2 Stereo saved on channel 6 came back
by itself.

1. **Alt+F4 stopped playing a pattern.** The F-key transport handler fired on F4 whatever the
   modifiers, so Windows' close chord started the transport instead of closing. Bare F-keys only
   now; any modifier falls through to the system (and into the close-question flow).
2. **VST FX on the MASTER bus.** `song.masterFxVst`, `track: -1` on the same four fxvst commands,
   spliced between the ported master chain and the safety limiter (`set_master_plugin_insert`) —
   so a mastering plugin hears the whole mix and the limiter still has the last word. The master
   strip in the mixer and the master view's panel both carry the slot. Wiring proven by an offline
   render test (a muting insert silences the mix; restoring brings it back).
3. **Why channel VST FX seemed to work only on plugin channels — fixed.** A VOLT-instrument
   channel is heard from the WEBVIEW, which the native insert cannot touch; only plugin channels
   were engine-only, hence the report. Now a channel with `fxVst` hands its PORTED instruments to
   the engine (silent browser voice, same mechanism as plugin slots) — live only, never in a
   render, never for unported instruments (dry beats vanished), and previews at rest stay
   browser-audible. Picking an FX also auto-enables transport-following, matching the synth picker.
4. **Duplicate plugins collapse to one row** (`vstDedupe`): keyed name+vendor, keep the current
   assignment if it IS one of the duplicates, else VST3 over VST2, else the numerically newest
   path (12.10 beats 12.7 — numeric compare, the string compare gets it backwards). Three
   WaveShells stop meaning three of everything.
5. **The mixer says what is on each channel**: an `mfx` badge per strip — enabled insert count +
   the VST FX name, full chain in the tooltip. The master strip shows its own.
6. **PLUGIN PARAMETER AUTOMATION.** The whole path: `ParamDesc` lists captured at load (VST3
   controller info filtered to kCanAutomate, VST2 names unfiltered — too many answer "no" for
   everything); `Plugin::set_param` (VST3: a real `IParameterChanges`/`IParamValueQueue` COM
   implementation in `vsthost/src/params.rs`, pooled, fed into `process`; VST2: the effect's own
   `setParameter` pointer); a `ParamMsg` through the node's port; `vst_param_set` /
   `fxvst_param_set` commands. Lane keys: **`vp<slot>.<paramId>`** (instrument plugin) and
   **`vf.<paramId>`** (the lane's channel FX; on an `M.` lane, the master's). Lane 0..1 IS the
   normalised parameter — no scaling tables. Throttled ~30 Hz per parameter with a trailing send.
   The door: **Ctrl+Space → "Plugin: automate a parameter…"** — pick the plugin (current
   instrument / channel FX / master FX), pick a parameter by name, get a lane in F7.
   Block-rate automation, applied at the engine's next block; sample-accurate ramps are a later
   refinement (the COM object already carries offsets).
   **Limits, said out loud:** the plugin's editor knob does not move with the lane (the
   controller half is not synced — audio only), and a `vf.` lane names its parameter only while
   the pick-time cache is warm; the id keeps automating either way.

## WHAT LANDED 2026-08-12 (second pass) — Esc, real meters, close-with-save, sidechain, VST FX

Mickey's next six, same day. All landed, 836 JS + 49 Rust checks green, exe shipped:

1. **Esc hides the plugin window it is typed into.** Intercepted in the plugin thread's message
   pump (`editor.rs::pump_messages`) — the keystroke lands on the plugin's own child window and
   would never reach our frame's wndproc. Root-window class check keeps it off foreign windows;
   the trade (a plugin cannot use Esc itself) is the trade every host makes. Hide, never destroy.
2. **Meters read past 0 dB.** All channel meter reads moved from byte data (which CLIPS at 0 dBFS
   — a +4 dB channel looked identical to a healthy one) to `getFloatTimeDomainData`; a channel
   over 0 paints its meter red for ~a second (`.mmeter.hot`). The master clip LED now sees true
   overs too.
3. **Alt+F4 / ✕ asks before closing an edited song.** Rust intercepts `CloseRequested`, the page
   answers: untouched → closes at once; edited this session → Save and close / Close without
   saving / Keep working. Two escape hatches, both deliberate: autosave has already run whatever
   is chosen, and a SECOND Alt+F4 within 10 s bypasses the question entirely, so a dead webview
   can never make the window unclosable (the husk lesson).
4. **The sidechain reaches the native engine.** `nengPattern` sends a `ducks` trigger list — one
   per note, INCLUDING notes the engine cannot play, so an unported instrument (and, once mute
   crosses, a ghost kick) still pumps the mix. `Mixer::duck` schedules the browser's exact
   envelope (down to `1-duck` at 6 ms, back up over `.03+duckRel*.5`) on the native strips'
   duck gains — which plugin channels sit behind, so **VSTs finally duck**. Panic resets them.
5. **VST effects on mixer channels — ROADMAP step 2, shipped.** One plugin effect per channel:
   the scan now lists effects (`kind:"fx"`, cache bumped to `vst-scan-4.json`), the Track panel's
   FX chain grew a "VST FX" slot (pick / window / remove; `ts.fxVst` in the song), and the engine
   hosts it as a node with a real INPUT (`Plugin::process_io` feeds the input bus both formats;
   `VstNode` grows an input when the plugin has one) spliced after the ported chain, before the
   drive (`Mixer::set_plugin_insert`). Editor windows share the plugin thread's registry at
   `FX_EDITOR_BASE + track`. **Proven by audio, per rule 1:** `examples/fxprobe.rs` renders a
   sine through the effect path offline — **WaveShell VST3 works**: SSLComp Stereo measurably
   compresses (0.500 in → 0.406 out), SSLGChannel unity, FabFilter Pro-Q 3 unity. The four Waves
   VST2 shells still refuse instancing (known, unchanged); the VST3 shells are the working door.
   Not yet: plugin FX state in the song (same gap as instruments), master-bus VST FX, PDC.
6. Duck/`fxVst` limits, stated: FX **automation** still tunes only browser nodes; the plugin
   effect is heard on the native side (always true for plugin instruments; for VOLT's own
   instruments it is audible via the engine's copy — "ASIO only" to hear it alone).

## WHAT LANDED 2026-08-12 — the "make it work right" day (panic, dropouts, startup, meters, FX)

Mickey's list, verbatim: a panic that actually silences; "the vst gone from some reason and then
it plays the note under a basic tool"; the taskbar icon parade at startup; no EQ visuals and no
audible insert effects on plugin channels. All five shipped, 848 checks green (JS 827 + Rust 21),
new exe delivered:

1. **Alt+P is a real panic now.** Stops the transport, cuts every webview voice (as before), then
   `eng_panic`: the engine cuts its voices and sweeps **note-offs for all 128 MIDI pitches into
   every plugin** (`VstNode::all_off` — the bookkept note-off list is exactly what can be wrong
   when a panic is needed). The engine must ANSWER within 800 ms; if it does not, it is wedged and
   the UI escalates to `eng_close` — the device is dropped out from under it, which is silence by
   disconnection — then auto-reopens. Tested from both ends (nengine.test.js drives the ladder,
   Rust owns the sweep).
2. **A plugin slot never falls back to the instrument underneath.** `trig()`'s silent-voice guard
   no longer requires `NENG.open` — engine down now means silent + a status line + **self-healing**:
   any "audio thread has stopped" marks the engine closed and schedules `nengAutoOpen` (once per
   30 s, so a persistent fault cannot loop). Also fixed the double-load race the log caught at
   20:58 (City Piano built twice into slot 8): concurrent `nengVstSync`s now join one in-flight
   `vst_load` per slot instead of each starting their own.
3. **Startup no longer parades every ASIO driver across the taskbar.** Root cause: cpal's ASIO
   enumeration LOADS each driver to ask its name, and startup enumerated everything up to three
   times (auto-open's device list, eng_open's label lookup). Now: device ids are **`HOST#hash`**
   (host in clear text, channel count no longer hashed — fork change in `io/cpal.rs`), the id
   lookup opens only the named host and matches by name, `eng_open` takes the label as a parameter
   and checks existence via the new `output_device_available` instead of enumerating, and
   `nengAutoOpen` goes straight at the remembered id. A WASAPI device now resolves with **zero**
   ASIO driver loads; an ASIO device loads only the drivers before it in registry order. Old saved
   ids don't resolve → one fallback enumeration on the first launch, then the new id is remembered.
   The full parade still happens exactly once per fresh cache in the device PICKER — listing is
   what listing costs.
4. **Meters and the spectrum panel see plugin channels.** `mixer.rs` grew the browser's own taps
   (AnalyserNode per strip off `duck`, one on the master off the limiter); `eng_meters` returns
   every strip's peak + one spectrum per poll; the UI polls at ~15 Hz while the engine follows the
   transport and **max-merges** into the existing meters and `SPEC` drawing — merged by frequency,
   not bin, because the two contexts can run at different rates.
5. **Insert effects are real on plugin channels — stage D is [~].** `native/engine/src/fx.rs`:
   **13 of the 15 `FX_TYPES` ported** (verb, echo, comp, crush, pulteq, softclip, flanger, chorus,
   autofilter, phaser, widen, superdrive, sausage), node-for-node with the browser's numbers.
   Chains rebuild only on shape change, tune in place otherwise (the reverb rebuilds itself when
   size/damp move — its impulse is baked into the convolver). `ts.fx` rides in the trackset it
   always travelled in; `masterFx` was added to the payload; knob drags and shape edits now call
   `nengMix()`. **Not ported, said out loud:** `limit` (Limiter L∞ — a hand-written look-ahead
   brickwall in a ScriptProcessor; its own careful port) and `pitch` (the delay-ramp shifter).
   An unported effect in a chain is skipped and reported once via `neng:err`, never silent.
   **Also not there yet: FX parameter *automation* on the native side** — lanes tune the browser's
   nodes live and do not stream to the engine, so an automated `fx0.mix` moves only the webview's
   copy. Base values follow every edit; automation is transport-rewrite territory.

Still unexplained (but now survivable): WHY the audio thread wedged on 2026-08-11 19:40. The
panic ladder and auto-reopen turn that from "Ctrl+R and lose the session" into two seconds of
silence, but the root cause — suspicion remains the render thread under heavy plugin load — is
still open, and the `attach:` log line will say whether a recurrence ever reached the thread.

## NEXT SESSION — start here (rewritten 2026-08-11, end of the mixer + plugin-window day)

Run `volt_status` (MCP) first, then read this whole section — it is thirty lines and it will save
you the four wrong turns this session took.

### Where things stand, in one paragraph

VOLT's desktop build hosts **VST2 and VST3 instruments**, opens **their own plugin windows**, and
has a **real mixer in Rust** — channel strips, sends, master bus, safety limiter — that the
browser's faders drive over IPC. **816 checks green** (`node tests/run.js`), all Rust tests green.
The last exe Mickey ran was healthy: plugins load in about a second, windows open by themselves,
the log is clean. What is NOT done: plugin patches are not saved into songs, meters still read the
browser rather than the engine, and two instruments (`pluck`, `psybass`) are still unported.

### The four rules this day cost the most to learn — all now in CLAUDE.md, none obvious

1. **`#[tauri::command]` runs on the UI thread.** Every slow one must be `(async)`. This froze the
   app for a whole day and moving work to a worker thread did NOT help, because the *command* still
   waits on the main thread.
2. **Plugins live on their own thread** (`native/desktop/src/plugins.rs`) — created, destroyed and
   shown there, with its own message pump. The audio thread has no pump (VST2 editors hang forever)
   and the main thread is the UI (a slow plugin freezes the window).
3. **A freeze is diagnosed by MEASURING, not reasoning.** Zero CPU = a lock; high CPU = a loop.
   `Get-Process` gives the first, `cdb.exe -pv -p <pid> -c "~*kn; qd"` gives the stacks. Two minutes
   of measurement beat three wrong hypotheses.
4. **A test that does not reproduce the threading proves nothing about the threading.** `editwin.rs`
   passed 50/50 while the app hung on the first plugin, because it used one thread for everything.

### How to build and give it to Mickey — the loop that works

He should not have to do the mechanics; do them for him.

```powershell
# 1. close VOLT gracefully (it autosaves), force only if it will not go
# 2. if the exe will not overwrite ("Access is denied") the file is LOCKED, not in use —
#    a terminated-but-unreaped VOLT still holds it. RENAME it out of the way; Windows allows that.
#    Do NOT keep closing VOLT in a loop: that mistake cost half an hour.
# 3. node tools/mkdesktop.js   (and VERIFY the hash matches beta2.html)
# 4. touch native/desktop/src/main.rs   (dist/ changes do not invalidate cargo's cache)
# 5. cargo build --release --features asio --manifest-path native\desktop\Cargo.toml
# 6. Start-Process the exe, and confirm it is alive and responding
```
**Every build closes his DAW**, so do not rebuild for cosmetics while he is working — stage the
change, compile-check it, and let it ride along with the next real one.

### Reading what happened on his machine

`%LOCALAPPDATA%\VOLT\volt.log` — every plugin load, graph attach, mixer route and editor window,
with the reason when one fails. It is the difference between diagnosing and guessing: it caught the
audio thread refusing plugin handovers, and it proved `vst_editor_open` had **never once been
called** when "the VST UI won't open" turned out to be a discoverability problem, not a bug.
Also on that machine: `vst-blacklist.txt` (plugins that killed a scan), `vst-scan-3.json` (the
plugin list cache), `vst-loading.txt` (present only mid-load; a leftover means that plugin killed
the last session and it gets blacklisted).

### 0. First, two minutes

`node tests/run.js` — expect **816, 0 failing**. If exactly one fails and it is
`metsync`'s *"a second measurement lands within alignment jitter"*, that is the **known flake**;
re-run it alone. Anything else is real.

### 1. ~~THE JOB: VST2 hosting.~~ **DONE 2026-08-10, and measured.**

*"need vst 2"* — built as decided, and the ship point holds: **a 64-bit VST2 sits in the
instrument list next to the VST3s, indistinguishable but for a label.** What exists now:

* `native/vsthost/src/vst2.rs` on the `vst` crate v0.4. One unified front door: `Module::open`
  picks the loader from the file (`.dll` = VST2, else VST3), `PluginInfo` carries
  `format: Vst2 | Vst3`, and one `Plugin` type processes both — `engine.rs`, `VstNode`,
  `nengPickVst` and `ins.vst` contain no format logic at all. The engine's one event list is
  translated to `VstMidiEvent`s at the last moment (`HostEvents::for_each_note`).
* **Counted for real by the working scanner (the earlier 232/114 estimate was across folders
  including (x86), which is never scanned): the three 64-bit VST2 folders hold 224 loadable
  64-bit DLLs → 214 plugins listed, 1 DLL 32-bit-skipped, 10 honest failures** (FL Studio's
  VSTi DLLs and Omnisphere.dll export no `VSTPluginMain`; the four Waves VST2 shells refuse
  instancing — Waves' VST3 shells already work). VST3 count unchanged: 1347 classes / 129 files.
  50 instruments across both formats, Diva 1.4.3, Sytrus, Slicex, the KORG collection, Kontakt
  and both FabFilter synths among them.
* **Proof by audio, per rule 1:** `vstprobe sytrus <wav>` renders IL Sytrus (VST2-only) at
  peak 0.79 / rms 0.15, and the measured first onset is 102.4 ms for a note scheduled at
  100 ms inside a 256-frame block — sound begins *after* the scheduled sample, never at the
  block boundary, so `delta_frames` placement is sample-accurate (the 2.4 ms is the synth's own
  attack). Released cleanly, `hard_exit` discipline unchanged.
* **Three plugins ended a scan and are permanently blacklisted**
  (`%LOCALAPPDATA%\VOLT\vst-blacklist.txt`, one path per line, hand-editable): Bionic Delay
  v1_2.64 **hangs inside `LoadLibrary` forever**, KORG LegacyCell segfaults on open, KORG
  WAVESTATION dies on open. This forced a defense worth having anyway: the desktop scan now
  writes a **journal** (flushed path before every open, deleted on completion), so a scan that
  crashes *or hangs and gets the app closed on it* auto-blacklists its killer on the next launch
  instead of wrecking every startup. `vstprobe` reads the same blacklist and states the count.
* The scan cache moved to `vst-scan-2.json` (shape grew: `{plugins:[{…,fmt}], skipped32}`), and
  the picker/splash/params panel say the format and the skipped count out loud.

### 2. ~~The plugin window.~~ **BUILT 2026-08-11.** `native/vsthost/src/editor.rs`.

The refactor it needed is done: `Plugin::take_editor()` hands the UI half out at load time, before
the plugin disappears into the audio processor, and `engine.rs` keeps it in a main-thread registry
(`EDITORS`) keyed by instrument slot. For VST3 that half is an add-ref'd reference to the same
controller the audio side holds — teardown is unchanged and still the audio side's job in the
documented order; for VST2 it is the crate's `Editor`, which hangs off the shared parameters
handle rather than the instance.

What it does: **"Open plugin window"** in the params panel of any plugin slot, plus a
command-palette entry and an Audio ▸ Plugins menu item. Both formats, one code path.

The four decisions that are not free to change (all in `editor.rs`'s header, and now in CLAUDE.md):
its own top-level window rather than a child of VOLT's (Tauri's tao/winit loop eats Tab, arrows,
Enter, Escape); created on the main thread via `run_on_main_thread` (a window's messages only reach
its creating thread, and that is the only one Tauri pumps); **closing hides, it never destroys** —
one view per plugin for its whole life, because a second `createView` crashes some of them; and
teardown is `removed()` → `setFrame(null)`, only if `attached()` actually succeeded.

**Proved, not assumed:** `examples/editwin.rs` runs the entire attach path — `createView`, the
platform check, `setFrame`, `attached`, `getSize`, and the teardown — against every instrument on
the machine, and never calls `show()`, so it is safe to run while Mickey is working. **50 of 50
opened a window**, each reporting its own real size rather than our 640×480 default: Kontakt
1213×942, Phase Plant 1385×894, Vital 1400×820, Diva 1200×670 in *both* formats, Serum, Sylenth1,
the KORG collection, the Waves instruments. The two that were silent all along are exactly the two
this unblocks.

**It also caught a shipped bug**, which is the point of having it: every VST3 was listed **twice**
in the picker, and one of the two could never load. A VST3 file exports its controller class next
to its audio class, identical in name and subcategories, and `vst_scan_now` filtered only on
`is_instrument()`. Now filtered on `category == "Audio Module Class"` (which `vstprobe` had from
the start), and the scan cache is bumped to `vst-scan-3.json` so the duplicate-ridden `-2` caches
on disk are discarded rather than trusted.

**The bug it shipped with, found by Mickey in ten minutes and fixed the same night.** VST3 editors
(Diva) opened fine; the first VST2 tried (Sytrus) **hung the whole app**. Cause: a VST2 plugin is
one object with no component/controller split and treats its creating thread as its UI thread —
and VOLT created plugins on the *audio* thread, which has no message pump, so `effEditOpen` waited
forever. The mirror rule turned out to hold for teardown too (Sytrus exits 222, Sylenth1
access-violates when dropped on the wrong thread). So now: **created on the main thread, moved to
the audio thread to play, posted back to the main thread to be destroyed** — which is exactly the
non-RT reclamation `01_ENGINE_ARCHITECTURE` in the bible describes, reached the hard way.

The lesson worth more than the fix: `editwin.rs` passed **50/50** while the app hung on the first
plugin tried, because it created and opened on one thread. `examples/editthread.rs` now reproduces
the app's *threading* and can run either arrangement side by side. A test that does not reproduce
the threading proves nothing about the threading.

**THE ACTUAL CAUSE, found after two wrong answers: `#[tauri::command]` runs on the main UI thread.**
Mickey's *"i do play and it hangs. i cant use the program"* was not the mixer and not the plugin
thread — it was that every Tauri command's body executes on the thread that draws the window,
because the macro's `execution_context` defaults to `Blocking`. So `vst_load` froze the UI for a
second per plugin, and **pressing Play loads the song's plugins first**, and `vst_scan` froze it for
a whole 350-DLL scan at startup — which the cache-name bump to `vst-scan-3.json` had just guaranteed
would happen. Every slow command is now `#[tauri::command(async)]`, which runs the same synchronous
body on the async runtime's thread pool instead of the main thread. Sixteen commands moved; the only
ones left on the main thread are the pure message-sends where ordering matters and nothing blocks.
**Moving work to a worker thread does not help by itself — the command still waits on the main
thread.** That is why the plugin thread alone did not fix it.

**And the bug before that one: the app stopped responding.** Moving plugin
creation to the main thread fixed VST2 editors and froze the window instead — instantiating a
plugin takes about a second, so a song with several of them holds the UI thread for seconds and
Windows paints it "not responding". Both threads VOLT had were wrong for the same reason from
opposite ends. There is now a **plugin thread** (`native/desktop/src/plugins.rs`) that owns
creation, destruction and every editor window, and runs its own message pump: slow plugins block
only it, the window keeps painting, the audio keeps playing. `01_ENGINE_ARCHITECTURE` in the bible
had this from the start — the instance manager and GUI bridge are their own domain — and it took
two wrong threads to read it properly.

`eng_mix` was hardened at the same time: it `try_lock`s the engine instead of blocking, because
`eng_open` holds that mutex for as long as a driver takes to start (up to 15 s) and a fader moved
during a device open would otherwise wedge a Tauri worker behind it. Skipping is safe **only**
because the mixer state is sent wholesale — the next message carries everything the dropped one did.

**Still to do here:** DPI scaling (`setContentScaleFactor`, on `IPlugView2`) is not wired, so on a
scaled display an editor may render at the wrong size. Also: the Waves instruments legitimately
appear more than once because three WaveShell versions are installed (12.7, 12.8, 12.10) — real
duplicates from real separate files, not the bug above, and not worth de-duplicating blind.

### THE WORK QUEUE — in the order I would do it

**1. Plugin patches saved into the song. Do this first; it is rule 3.**
The *assignment* is saved (`ins.vst`) but the *sound* is not, so reopening a song gives you the
plugin's default patch back. Mickey now has a working plugin window, which means he can *build* a
sound — and every one he builds is thrown away when he closes the song. That is the unforgivable
failure wearing a different hat, and it gets worse every day it is not done.
`native/vsthost/src/stream.rs` already does the byte-level work: `getState` on save, base64 into
the `.volt`, `setState` on load. Key the state by **class ID, not path or name**, or a vendor update
orphans it. The plugin thread owns the plugin, so state goes through a `PCmd`.

**2. ~~Meters from the native engine.~~ DONE 2026-08-12** (polled `eng_meters` rather than an
event — the UI asks at drawing rate for what it is actually looking at; see "WHAT LANDED" at the
top). Master gain-reduction readout from the native limiter remains unbuilt.

**3. Port `pluck` and `psybass`.** The last two instruments `trig()` has and the Rust engine does
not, so under "ASIO only" in pattern mode they go **silent** — which reads as VOLT being broken.
The parity harness grades them the same way the other twelve were graded; the bar is a null test at
the noise floor.

**4. Then the big one: the native engine becomes the transport.** Everything above is preparation
for it. See "SIMPLIFY — one engine, like the big guys" below; that section is still the plan and is
still right.

### Known-outstanding, smaller

* **One unexplained fault, instrumented but not solved.** On 2026-08-11 the audio thread stopped
  answering `Cmd::VstAttach` — ten `vst_load: the AUDIO THREAD never answered in 30 s` in the log —
  after a song with **eight plugins including three Diva instances** was loaded. The retry storm it
  caused is fixed (a failed plugin is remembered, not rebuilt on every Play), and `attach:` lines
  now log arrival at the audio thread so a recurrence says whether the command ever landed. The
  root cause is still unknown; suspicion is the render thread wedging under that plugin load.
* **DPI scaling for plugin editors** (`setContentScaleFactor`, `IPlugView2`) is not wired, so on a
  scaled display an editor may render at the wrong size.
* **The song-mode exception in `nengSpk` is stale.** "ASIO only" refuses to mute the webview in song
  mode, on the grounds that the engine could not follow a song order — it can now, via
  `nengSongStep`. That is why song mode sounds doubled. Revisit when the engine owns the transport.
* **The log's timestamp fix is staged but unbuilt** (local time instead of UTC in `log.rs`) — it
  compiles; it just was not worth closing his DAW for on its own.

### Working rules — the expensive ones

* **Never drive VOLT with synthetic keystrokes.** Every letter is a note; three landed in his dnb
  song before it was caught. Verify UI in the headless harness (`tests/nengine.test.js` has the
  Tauri stub), not by automating his desktop. Do not screen-capture his machine either.
  **Launching, closing and building for him is fine and expected** — he asked for it directly
  (*"why dont u lauch it yourself?"*). Only *using* the UI is off limits.
* **`volt_build desktop` was silently building stale HTML** — twice over: the staging step did not
  run, and changing `dist/` does not invalidate cargo's cache. Both fixed in `tools/volt-mcp.js`
  (content-hash check + touching `main.rs`), and the fix is live **only after an MCP restart** —
  **which a /clear does NOT provide.** That assumption shipped Mickey a broken app: the exe paired
  new Rust with the previous night's page. Verify after every desktop build that
  `dist/index.html` hashes equal to `beta2.html`.
* **Put a test assertion in the same block as the `page.evaluate` that produced its data.** Two
  separate bugs this session came from appending assertions after an unrelated evaluate, so they
  silently read a different result object and "passed" or failed for the wrong reason.
* **Rust block-buffers stdout when redirected** — a harness that crashes loses everything it
  printed and appears to have died a step earlier. Flush every line in any diagnostic tool.

## SIMPLIFY — one engine, like the big guys (set 2026-08-10, night)

Mickey, after actually using plugins for an evening: *"we need to simplify all the vst and engines
and shit like the big guys."* Every complaint from that session is one seam of the SAME split —
two engines pretending to be one DAW — so the answer is not seven patches, it is one direction:
**the native engine becomes the only engine.** Until it is, each seam gets named honestly:

| what he hit | why it happens | the fix |
|---|---|---|
| YouTube stutters when VOLT starts | ASIO4ALL takes the sound device EXCLUSIVELY — that is what makes it fast. Every DAW on ASIO4ALL does this. | Real fix: an interface with its own ASIO driver (the nio's, via UMC/Novation drivers) shares better. Not a VOLT bug. |
| slow to be ready | ~2 s ASIO open + first plugin scan | **DONE tonight: splash** shows both steps and clears itself; click through it to start earlier. Scan is disk-cached, so only the first run is slow. |
| big latency on Diva | while BOTH engines play, the plugin is deliberately aligned to the slower one (~58 ms) so they groove together — the flam fix works by delaying ASIO. | "ASIO only" gives the true 5.8 ms today. Permanent: one engine. |
| row 0 didn't play | the engine joins the browser mid-lookahead (~2 rows in), so the first pass has a 2-row hole; those rows sound from loop 2 onward | structural until the native engine owns the transport — then there is nothing to join late. |
| pattern mode only | `nengFollowTransport` refuses song mode because the engine has no song structure yet | part of the transport work below. |
| browser storage in Projects panel | the desktop build still stores like a browser tab; the song-folder mirror writes real files but the panel shows the tab's world | its own item below — a desktop app should read a folder. |
| old VST2s | not built yet | **DONE 2026-08-10** — 214 of them in the picker, same list as the VST3s. |

**So the order is now:**

1. **The native engine becomes the transport** (pattern first, then song mode). `pump()` stays for
   the browser build; in the desktop build the engine owns the clock, plays ALL ported instruments
   + plugins, and the webview draws. This kills: the flam logic, the row-0 hole, the alignment
   latency, the drift, "ASIO only" as a compromise, and pattern-only plugins. It needs: the
   remaining 8 instruments ported (parity harness exists), the mixer stages B–C, and song-order
   playback in the clock. It is the biggest item and it is worth more than everything else combined.
2. ~~**The plugin window.**~~ **DONE 2026-08-11** — see "NEXT SESSION" item 2 above.
3. ~~**VST2.**~~ **DONE 2026-08-10** — see "NEXT SESSION" item 1 at the top for the full record:
   214 VST2 plugins listed and loadable behind the same one `Plugin` type as VST3, Sytrus proven
   by rendered audio with sample-accurate note placement, three scan-killers blacklisted with a
   journal that auto-blacklists future ones. The 32-bit bridge remains a separate later decision
   (in the folders VOLT scans it turned out to matter for exactly 1 DLL; the old 114 count was
   dominated by `Program Files (x86)`, which is never scanned).
4. [~] **Desktop storage: files, not browser storage.** FIRST HALF DONE 2026-08-10 (night): the
   Projects panel on desktop now leads with **"Your songs" — the actual .volt files in the song
   folder**, newest first, click to open (same code path as Open…), refreshed on the panel's tick.
   Browser storage is still listed but labelled **"old world"**, with the hint saying plainly that
   the files are the home now. Covered by 4 checks in `tests/nengine.test.js`.
   Still to do to finish the kill: new songs should CREATE their file immediately (today the file
   appears on first autosave), closing a tab should point at the file not the localStorage copy,
   and the localStorage autosave demotes to crash-recovery only. Samples in OPFS by hash stay —
   that part is right regardless of where songs live.

## HOW TO PLAY A VST, today (written 2026-08-10, after "i dont know how to operate this")

The honest state: **plugins play alongside your drums, their own window opens when you add them,
and the mixer channel next to their notes controls them.** What is still missing is that the patch
you build is not saved into the song: reopening gives you the plugin's default again. That is the
next job.

1. **Open `volt-desktop.exe`.** The audio interface opens itself (top bar shows `ASIO … 5.8 ms`),
   and the plugin list scans itself in the background. If the badge says `ASIO: off`, click it and
   pick ASIO4ALL once — it is remembered from then on.
2. **`Ctrl+Space` → type `add` → "Add a VST instrument…"** and pick a synth. Since 2026-08-10 the
   list holds the 64-bit VST2s too — Sytrus, Slicex, the KORG collection, the FabFilter synths —
   with the format shown on each row; they load and play identically. It lands at the BOTTOM of the
   instrument list as its own new instrument with an amber badge — nothing you already had is
   touched. Following the transport switches itself on.
3. **Its window opens by itself** — that is its real interface: browse presets, turn knobs. **This
   is what Kontakt and Phase Plant were waiting for**; they were silent because they start empty and
   this is the only way to give them something to play. To bring the window back later, **click the
   amber plugin badge in the instrument list**, or use the button in the params panel, or
   `Ctrl+Space` → "plugin window". Closing it just hides it: re-opening is instant and the sound is
   exactly as you left it for the session.
   (Until 2026-08-11 none of this was discoverable — the window worked and nothing ever asked for
   it. The log showed `vst_editor_open` called zero times across three sessions of loading plugins.)
4. **Put notes on any track with that instrument selected**, exactly like any other instrument.
5. **Press Play.** Drums come from VOLT, the synth from ASIO, and they start *together* (the flam
   is fixed). Over a long session the two clocks can still drift apart by a little — that dies when
   the native engine plays everything.
6. Want the plugin alone, clean? `Ctrl+Space` → "ASIO only" mutes VOLT's side.

To change or remove the plugin: click its instrument, the params panel shows the plugin with
**Open window / Change / Remove** buttons.

**The one thing to know:** the patch you build in a plugin's window is NOT saved into the song yet.
It survives as long as the app is open; reopening the song gives you the plugin's default sound
back. That is the next job.

**What is NOT there yet, so nobody hunts for it:** plugin settings saved into the song's sound (the
*assignment* is saved; the patch is whatever the plugin wakes up with — **this is the next job**),
VST effects on channels, DPI scaling for editor windows on a scaled display, and song mode through
the engine. That is the order of the work below.

## A menu bar — done 2026-08-10

Mickey: *"cant you add a bar with file/preferences and stuff so it traditionally be possible to get
to audio devices and stuff?"* Fair. VOLT has always been a palette-and-hotkeys program, which is
fast once you know it and a locked door if you don't.

`#menubar`: **File · Edit · View · Track · Transport · Audio · Preferences.** 59 entries, and every
one of them is an **existing command referenced by name** — no second implementation of anything.
That buys the guarantee worth having: rename a command and the entry renders greyed and labelled
"(missing)" rather than quietly doing nothing, and `tests/menu.test.js` fails with the name.

- **No Alt accelerators, on purpose.** Alt+F, E, V, S, T, H and B are Firefox's own menu mnemonics
  and the page cannot cancel them (see CLAUDE.md). Click to open, slide across to move between
  menus, Escape or click-away to close.
- `mousedown` preventDefaults so the bar can never take the keyboard away from the grid — VOLT's
  hotkeys die the moment focus lands somewhere unexpected, and the suite asserts it.
- Desktop-only commands are filtered exactly as the palette filters them, so a browser tab never
  offers the ASIO device picker.
- The audio device now has three doors: the menu, the palette, and the Settings gear.

13 checks in `tests/menu.test.js`.

## Two GUIs that were overdue — done 2026-08-10

Both were `prompt()` with a numbered menu, or nothing at all. `askPick()` is a third dialog shape
alongside `askConfirm` and `askText`: a filterable list with arrow keys, Enter and Escape.

- **"Native engine: choose output device…"** is now a list. ASIO first, each row saying which API
  it is (the same interface appears under both and they are not the same latency), the open device
  marked, type to filter. It used to be *"type the number of your audio device"*.
- **"Open project…"** — the other half of the song folder, which until now had only ever been
  written to. `lib_write` has mirrored every song to `Documents\VOLT` since the desktop build
  existed, and `lib_list`/`lib_read` were implemented and tested in Rust and **never called from
  the UI** — songs went in and the only way back out was Explorer. Now there is an **Open…** button
  in the toolbar (desktop only) and a palette entry, listing songs newest-first with when and how
  big. In a browser tab it falls back to the file picker, so the verb works in both builds.
  Drag-and-drop and Import accept `.volt` as well as `.json` now, since that is what the folder
  writes.

Both go through `newTab()`, the same door Import and drag-and-drop use, so a song opened from the
folder cannot behave differently from the same song dragged in. 10 checks; the suite drives the
real dialogs with real key events rather than calling the underlying functions.

## "HOW DO I KNOW IT IS GOING THROUGH ASIO?" — the answer, and what it exposed (2026-08-10)

Mickey, after trying it: *"i dont see any way to stream directly the sound via the asio shit... i
actually tried to choose another audio interface i have and i still heard sound through the same
headphones."*

**He was right, and the reason is worse than a missing readout: both engines were sounding.** ASIO
out of the interface he picked, and the webview out of whatever Windows calls default — his nio. Of
course changing interface did not change what he heard. Nothing on screen said any of this, so
there was no way to work it out from inside the app. That is a design failure, not a user error.

Fixed, minimally and where he will actually see it:

- **An ASIO badge in the toolbar**, desktop only, with three states and the device name and
  measured latency in it: `ASIO: off` (grey — everything you hear is the webview), `ASIO +web`
  (amber — the engine is open but the webview is audible underneath, which is the state that
  confused him), `ASIO ONLY` (teal, glowing — the webview is muted and what you hear is the
  interface, alone). Its tooltip says the same thing in words. **Clicking it opens the device
  picker**, so the thing that tells you the state also changes it.
- **The hang was two processes.** One `volt-desktop.exe` not responding and holding the ASIO
  driver, so the second could never open it and sat there. Same undead-ASIO4ALL shutdown recorded
  in `native/README.md`, now hitting the app rather than the CLI. `taskkill` does not clear them;
  PowerShell `(Get-Process volt-desktop).Kill()` does. **This needs a real fix** — the app should
  refuse to start a second instance, and should time out and say so rather than hanging.
- **Right-click no longer opens WebView2's menu** in the desktop build (kept in text fields, where
  Cut/Copy/Paste is genuinely useful, and left alone in a browser tab where it is the browser's).
- **"in this browser"** retitled in the desktop build — it is a window with an icon and a song
  folder, not a browser.

### "the audio thread has stopped" — the real bug behind it, fixed 2026-08-10

Mickey, playing his own song: *"native engine: the audio thread has stopped"*, and no ASIO icon in
the taskbar. Those two symptoms are the same event.

**`trig()` panics on a missing parameter.** `Job::p()` is deliberately loud — a silently-defaulted
parameter is the one bug a null test can never catch, because both engines still make *a* sound.
Correct for a render binary. **Fatal in a live audio thread**: the panic unwinds, the thread dies,
its `AudioContext` drops, the ASIO driver is released (hence no tray icon), and every command after
it reports "the audio thread has stopped" with nothing to say why. **One instrument in one song
whose parameters do not match the port took the whole engine down, permanently.**

- The `trig()` call is now inside `catch_unwind`, and the panic is demoted to the same reported
  error a missing sample gets. `p()` stays loud where loud is useful.
- **The engine can now talk.** `neng:err` carries anything it refused to play to the status line —
  deduplicated on the Rust side, so a bad instrument on every row of a looping pattern is one line
  per take, not sixteen a bar. Before this it went to `eprintln!`, i.e. a console nobody has open,
  and an unplayable note was simply a hole in the music.
- If the thread does die, the badge stops claiming a device is open and the message says what to do
  instead of quoting a channel error.
- Two Rust tests pin the behaviour: `p()` must keep panicking (or the `catch_unwind` is dead code),
  and its message must name the instrument and the parameter, because that string is now what
  reaches Mickey's status line.

### The hang itself — fixed 2026-08-10
- **One VOLT at a time.** `tauri-plugin-single-instance`, registered *first* in `main.rs` because it
  decides whether the process lives at all. A second launch raises and focuses the window you
  already have instead of starting a rival for the sound card. An ASIO driver owns the hardware
  exclusively, so a second instance could never have opened the device the first one held — and
  when the first has hung, the second looks broken for reasons nothing on screen explains.
- **`eng_open` gives up after 8 s, not 20**, and the message names the actual cause: something else
  is holding the driver — another DAW, or a VOLT that did not shut down cleanly — with the
  PowerShell line that clears it. A driver that is going to open opens in under a second; one that
  is going to hang hangs forever, and twenty seconds of frozen window teaches the wrong lesson.

### Still to do, and asked for directly
- **"Show ASIO panel"**, like FL Studio's. Every ASIO driver exposes `ASIOControlPanel()` — buffer
  size, sample rate, its own routing. cpal does not surface it, so this is a small addition to the
  vendored fork plus a Tauri command. **This is the next native task**, because buffer size is the
  other half of latency and there is currently no way to touch it.
- An input-device list, once recording through ASIO matters.

## THE BIG ONE — a full DAW with VST support. Build order (set 2026-08-10)

Mickey: *"PLEASE MAKE THE MIXER FULLY VST CAPABLE - LETS TAKE IT TO THE NEXT LEVEL - make it full
blown daw that works with vst and be pro"*.

That is the goal now. It is **months of work, not a session**, and the honest thing is to say so and
then lay it out so each step ships something usable rather than disappearing into a rewrite. Read
the section below this one first — it explains why the mixer, not the plugin format, is the blocker.

### COURSE SET 2026-08-10 (evening): the old DAW is the reference, not the road

Mickey, after stage A landed: *"i dont want the old daw get on our way to execute the real daw i
want with vsts up and running"*. He is naming a real trap, and the plan changes shape because of it.

The trap: stage A's null test found that **Chrome's compressor is not the spec's**, and the plan's
reflex was to port Chrome's private algorithm into the crate before building anything else (A2
below). That is the old DAW steering the new one — weeks of work whose only output is *replicating
a browser quirk*, with the VST host still not started at the end of it. Follow that reflex
everywhere and the native DAW ships in a year, a perfect museum copy of a browser tab.

So, three principles, set now:

1. **The browser build is the reference player, permanently.** It is frozen-in-amber good at one
   thing: playing every song Mickey ever saved, exactly as it sounded. It keeps doing that forever
   — GitHub Pages does not rot. Rule 3 ("don't break old songs") is satisfied by the browser build
   *existing*, not by the native engine imitating its bugs.
2. **Parity is a tool, not a gate.** Null tests stay — they are how a *port* is proven correct, and
   they just caught four real bugs. But "matches Chrome" is no longer the bar for shipping native
   work. The native bar is: **spec-correct where the spec speaks, and judged by an A/B render of a
   real song where it doesn't.** When native differs from Chrome because Chrome is wrong (the
   compressor, the mono upmix), native keeps the spec behaviour and the difference gets *measured
   and written down*, not eliminated.
3. **VST work starts now, not after the mixer is finished.** The decided order (section 0) was
   already "instruments first, they need no mixer" — and stage A built the insert slot effects
   need. Nothing about the remaining mixer stages (sends, master bus) blocks a plugin making sound.

**The queue, re-cut accordingly — VSTs to the front:**

1. [x] **VSTi host — DONE 2026-08-10, same evening.** `native/vsthost` (scan/load/COM) and
   `native/engine/src/vst.rs` (a plugin as a node in the graph). **Vital, Diva and Jup-8000 V play
   from VOLT's own clock, through ASIO4ALL, at 5.80 ms** — peak measured at the plugin's output, not
   assumed. `live <device> vst <name>`. Details in the "VST3 host" section below.
1b. [x] **And it is in the tracker, not just the command line — DONE 2026-08-10.**
   **Ctrl+Space → "VST3 instrument for this slot…"** picks a plugin for the current instrument, from
   the same list dialog the device chooser uses. Then type notes in the grid and play with the
   native engine following the transport: that instrument's notes go to the plugin, everything else
   still plays VOLT's own way.
   - **The slot number IS the instrument index**, on both sides. One numbering, so the bug where the
     wrong synth plays cannot happen.
   - `ins.vst` lives on the instrument, so **the song file remembers its plugins**. The engine does
     not — plugins live on the audio thread and a new device builds a new one — so `nengVstSync()`
     reloads them on the way into Play, the same way samples are re-sent.
   - Stop reaches inside the plugins: a VST has no idea there is a transport, so a note-off it never
     gets is a synth droning until the app closes.
   - **Closing the app now ends the process by force** (`main.rs`), because Windows' DLL teardown
     runs plugin shutdown code that hangs (Jup-8000 V) or crashes (Vital, Diva, Kontakt).
   - Tested from both ends: 11 new checks in `tests/nengine.test.js` assert what the UI sends
     (slot, `instr:'vst'`, no ITYPES params, plugin loaded *before* the pattern), and two Rust tests
     assert the engine reads exactly that — the serde mismatch is the only failure this wire has.
   - **Not yet:** the plugin's own window (so presets are unreachable — see step 3), per-note
     velocity curves, parameter automation, and song mode.
2. **One VST3 effect in the strip** — the `fx_in`/`fx_out` slot in `mixer.rs` exists as of stage A.
   *Ship: Pro-Q 3 on a channel, heard through ASIO.*
1c. [x] **It starts itself, like a DAW — DONE 2026-08-10.** Mickey: *"the asio engine should run when
   starting the program with searching all the vsts like other daws do why should i do these steps
   like a bitch"*. He is right, and the steps are gone:
   - **The audio device opens itself.** The chosen device is remembered (`volt.neng.dev`) and
     re-opened at startup. It only ever reopens a device he picked; if that interface is gone it
     says so once and stops rather than grabbing some other output.
   - **The plugin list scans itself**, in the background, at startup — and is cached **on disk**
     (`%LOCALAPPDATA%\VOLTst-scan.json`), so it is only genuinely slow the first time after a
     plugin is installed. The picker reads the warm list instead of opening 129 DLLs while somebody
     waits. `vst_rescan` forces a fresh scan.
   - Still a deliberate step: **"follow the transport" stays off by default.** Turning it on is not
     a preference, it is a change of engine — the native side has six instruments and plugins, so
     defaulting it on would quietly stop playing most of what is in his songs.

2b. [x] **`IEditController`** — DONE. Created, connected, state transferred. It did NOT make the
   twelve silent instruments sound (they want a preset, not a controller — see below), but it is
   the object the parameter panel and plugin state both need, and it is in place.
1d. [~] **The two engines now START together — DONE 2026-08-10 (late). Drift over minutes remains.** Mickey, first time he got
   Diva playing in a real pattern: *"i tried to run all channels together they not running
   together."* He is hearing exactly what the architecture currently does.
   - **Why.** With "follow the transport" on, two engines play the same pattern: `pump()` through
     the webview at **~58 ms**, and the Rust engine through ASIO at **5.80 ms**. Both start when
     Play is pressed, so the plugin lands about **50 ms early** — a 32nd note at 174 bpm, which is
     a flam, not a groove. Nothing is drifting yet; they simply start in different places.
   - [x] **The cheap fix is in.** `nengFollowTransport` aims the engine at the browser's next
     unscheduled row: `start_in = (SEQ.next − now) + browser latency − ASIO latency`, walked
     forward a row at a time if that lands under 50 ms, and `from_row` says which row that is —
     because by the time samples have uploaded, `pump()` has already booked its 160 ms lookahead
     and "start at row 0" would be starting in the browser's past. The engine's fixed 0.15 s
     start survives only for payloads that don't carry the fields. Asserted from both sides:
     the JS sends it (`nengine.test.js`), Rust reads it (`an_aligned_start_crosses...`).
     Residual error is the IPC hop, a few ms.
   - **What it will NOT fix:** two devices means two crystals, so a long take will slowly drift
     even after the starts are aligned. Good enough for writing; not good enough to bounce.
   - **The real fix is the one already on this list:** the native engine plays *everything* and the
     webview plays nothing — one clock, no alignment problem, and "ASIO only" stops being a
     compromise. That needs the mixer (stage B–D) and the remaining instruments.
   - Until one of those lands, the honest way to use a plugin is **"ASIO only"** with a pattern
     that is only plugins, or accept the flam while writing.

2c. [ ] **The plugin's own window — the next real job, and it needs a refactor first.**
   Researched 2026-08-10 (Steinberg SDK headers, dev portal, `editorhost.cpp`, plus how JUCE and
   Ardour actually do it). What matters before writing any of it:
   - **The controller has to live on the UI thread, and right now it does not.** `Plugin` owns both
     halves and the whole thing is moved into the audio processor, so nothing on the main thread can
     reach the controller to open a view. Splitting component (audio thread) from controller (UI
     thread) is the prerequisite, and it is the same split every host makes.
   - Call order: `createView(kEditor)` → `isPlatformTypeSupported("HWND")` → `setFrame(our
     IPlugFrame)` → `attached(hwnd, "HWND")` → `getSize`/`onSize`. **`createView` hands back a view
     with a refcount of 1 — do not add-ref it.** Never call `removed()` unless `attached()`
     succeeded, and tear down as `removed()` → `setFrame(null)` (the SDK sample does it the other
     way round; JUCE and Ardour both do this order).
   - **Never create a second view for one plugin** — it crashes some of them.
   - `IPlugFrame::resizeView` is required for resizable editors, and needs a re-entrancy guard: the
     plugin calls it *from inside* `onSize`. `setContentScaleFactor` must sit OUTSIDE that guard or
     the resize it triggers gets swallowed and the UI renders at the wrong scale.
   - **`kResultTrue == 0` on Windows.** `if (view.onKeyDown(..))` reads backwards.
   - Plugins install their own keyboard hooks; hosts forward no keys on Windows. And **do not run
     `IsDialogMessage`/`TranslateAccelerator` over messages bound for the plugin's window subtree** —
     it eats Tab/arrows/Enter/Escape and `onKeyDown` cannot recover them. Tauri's message loop
     belongs to tao/winit, so what it does with accelerators has to be checked before parenting a
     plugin editor into one of its windows.
   - Worth doing at the same time: **read `moduleinfo.json`** (`Contents/Resources/` first, then
     `Contents/`) to list a module's classes *without loading the DLL* — cheaper and safer than the
     current 129-DLL scan — and a **"dead man's pedal"** scan log (write the path before
     `LoadLibrary`, erase after; a leftover entry names the plugin that crashed).
   - **Key plugin state in a song by class ID, never by path or name**, and honour
     `moduleinfo.json`'s `Compatibility` block, or songs drop plugins after a vendor update.

3. **Plugin state in `.volt` files + generic param panel** — before anything else grows, because a
   song saved with a plugin that opens wrong is the one unforgivable failure, native or not.
4. **Mixer stages B–D** (sends, master chain, IPC, ported FX) — at the new bar: spec-correct nodes,
   graded by rendering a real song through both engines and comparing LUFS/spectrum/by ear, null
   tests only where both sides implement the same spec.
5. **Editor windows, automation, latency compensation** — then the transport itself.

**Demotions, explicit so they stay demoted:** A2 (porting Blink's compressor) and the oversampler
gap are now **compat items, done only if a real song A/Bs wrong through the native mixer** — the
measurements in stage A's notes stay pinned in `tests/parity.test.js` as known gaps so they cannot
quietly worsen. The reverb-IR seeding stays (it is one line and benefits the browser too).

**Do them in this order. Each one ends somewhere you can make music.**

### 1. Native mixer in Rust — the actual blocker (biggest single job)
Today the engine plays voices straight to `destination()`. There is no channel, no EQ, no send, no
master chain — all of that is `buildGraph()` in the browser. A VST effect has nowhere to live until
this exists, and neither does a real fader. Detailed plan set 2026-08-10, after Mickey asked to
plan it properly:

**The one architecture decision, made:** the mixer is built from the vendored
`web-audio-api` crate's nodes — the same graph `trig()` already plays into — not hand-rolled DSP.
Three reasons. (a) Every node the strip needs already exists in the crate (gain, biquad,
waveshaper, dynamics compressor, stereo panner, delay, convolver, analyser), so this is
transliteration with a null test, the same discipline that got 12 instruments to −118 dB.
(b) The browser strip's sound *is* those nodes — a hand-rolled mixer could only approximate it,
and rule 3 says old songs must keep sounding right. (c) The crate has a custom-node path
(`AudioProcessor` trait + its worklet module, verified in `native/vendor`), which is exactly
where a VST3 `process()` call slots in later — **a VST becomes a node in the strip**, not a
second audio system bolted on.

**Stages, each ending somewhere usable:**

- [x] **A. One channel strip, nulled.** **DONE 2026-08-10.** `native/engine/src/mixer.rs` — the
  full chain, in `ensureChans()`'s order: in-gain → EQ shelf/peak/shelf → lowpass → fx insert (two
  gains wired straight through, and the slot a VST goes in) → shaper → compressor → panner → fader
  → duck, with both send taps. `applyMix()`'s body is `Channel::apply`. The browser half of the
  null test calls the REAL `ensureChans()` and `applyMix()` rather than a copy living in the test.
  **It nulls at -71 to -86 dB** — the 16-bit floor — for flat, EQ, filter, pan and everything-at-
  once. Seven strip jobs in `tests/parity.test.js`; suite total **730 checks, 0 failing.**

  Four things were wrong, none of them in the strip, all found by the null test:
  1. **The crate's compressor rounded its 6 ms lookahead up to whole render blocks** — 384 samples
     instead of 264, 2.7 ms of latency nobody asked for, and it made a null test impossible.
     Browsers use `floor(sample_rate * 0.006)`: measured in Chrome with an impulse, 264 at 44.1 kHz,
     288 at 48 kHz, 576 at 96 kHz. `VOLT FORK` in `dynamics_compressor.rs`, now a sample delay line.
  2. **Chrome's compressor turns a mono signal stereo and the spec says it should not** — and the
     panner is next in the chain, which applies the *mono* equal-power law (-3 dB) to a mono input
     and unity to a stereo one. Every VOLT instrument is mono, so this was 3 dB on every note. The
     strip now forces the compressor to two channels explicitly.
  3. **The crate refused to reassign a waveshaper curve** (`InvalidStateError - cannot assign curve
     twice`). Browsers allow it and the drive knob needs it — `applyMix` rebuilds the curve every
     time the knob moves. `VOLT FORK`, plus `clear_curve()` for `curve = null`.
  4. The strip's own default filter (18 kHz, Q 0 dB) sits close enough to Nyquist to **boost peaks
     by 1.8x** while cutting energy. Nothing to fix — it is what the browser has always done — but
     it is why the test signal has to be quiet or the 16-bit file clips and grades a clipper.
- [~] **A2. DEMOTED 2026-08-10 (evening) — see COURSE SET above.** Was: "port Blink's compressor
  before B." What the null test established, kept for the record because the numbers stay pinned in
  `tests/parity.test.js` (`probe-comp-unity` / `probe-comp-hit`): **Chrome's DynamicsCompressor is
  not the spec's algorithm** — not unity at ratio 1:1 (a ~2 dB dip for its first ~90 ms, then
  agreement at -87 dB), and 1–2.5 dB more gain reduction when compressing hard (`strip-comp`,
  -9 dB). The crate implements the spec faithfully; the native mixer keeps the spec behaviour.
  Porting Blink's kernel happens **only if a real song A/Bs wrong** through the native master bus.
  Same status for the smaller 2x-oversampler gap (`strip-drive`, -21 dB: Chrome's FIR vs the
  crate's FFT resampler; nulls on a sine, parts company on broadband material).
- [x] **B. Sends and master chain. DONE 2026-08-11.** `Master` in `mixer.rs`: ping-pong delay bus
  (hp 280 → two delays → ±0.65 pan → 0.9 out, 0.42 feedback), reverb bus (hp 300 → convolver with
  the seeded 1.9 s impulse → 0.7 out), master gain → master FX insert → safety limiter → `spk_out`.
  `Mixer` ties strips to it and `ensure(n)` grows them like `ensureChans(n)`.
  The prerequisite is done too: the reverb IR is seeded (`0x2EFBAD`, one generator across both
  channels), so the browser's renders are reproducible for the first time and the bus can be
  compared at all.
  **Five new parity jobs** (`master-flat`, `-vol`, `-delay`, `-reverb`, `-limit`). **None of them
  null, and that was the plan** — this stage's own note said "an A/B render where they don't (the
  limiter)". The master chain ends in a DynamicsCompressor and Chrome's is not the spec's (A2), so
  every job carries that gap. What each now asserts instead is the thing a null test cannot say and
  a mixer actually cares about: **the mix comes out at the same level.** Measured ratio between the
  engines, everywhere the limiter is not reacting: **0.978 — a steady -0.19 dB**, pinned at ±0.5 dB.
  The reverb job lands at the same -30 dB as flat and no worse, which is the evidence the convolver
  and its seeded impulse agree over 167,580 samples. The delay job's wider gap was chased to its
  cause rather than assumed: the repeats push peaks past the limiter's -3 dB threshold, so it runs
  in its compressing regime there (envelope ratio holds 0.978 away from the repeats, swings to 1.19
  exactly on them). A unit test —
  `mixer::tests::a_delay_in_a_cycle_repeats_where_it_was_told_to` — rules out the other suspect by
  proving the crate's delay-in-a-feedback-loop repeats exactly on time rather than charging a render
  quantum for the cycle.
- [x] **C. Mixer state over IPC — the ship point. DONE 2026-08-11.** `nengMix()` mirrors
  `applyMix()` and is called from it: the whole `song.trackset` plus master volume and the
  tempo-derived delay time, wholesale on any edit, **throttled to one message per 40 ms** because
  `applyMix` also runs from the mix LFOs at frame rate. `eng_mix` applies it to every strip.
  Opening a device pushes the mix immediately, and the last snapshot is remembered in Rust so
  switching devices mid-session does not silently reset the mix to unity. Covered by 8 checks in
  `tests/nengine.test.js`. **Faders, EQ, filter, pan, sends and drive now do something through
  ASIO.**
  **Still to do from this stage:** meters the other way — the engine publishing per-channel peak
  and master gain-reduction on a Tauri event for the browser to draw.
- [~] **D. Insert FX. 13 OF 15 DONE 2026-08-12** — `native/engine/src/fx.rs`, node-for-node
  (see "WHAT LANDED" at the top). Outstanding: `limit` (the ScriptProcessor look-ahead brickwall —
  needs a custom AudioWorkletProcessor and its own null test) and `pitch` (the delay-ramp
  shifter), plus browser-parity jobs in `tests/parity.test.js` — today's proof is Rust-side
  behavior tests (echo timing, bypass-is-a-wire, tune-vs-rebuild), not a null against Chrome.
- [ ] **E. The VST insert point.** A `Vst3Node` implementing the crate's `AudioProcessor`,
  sitting in the fx insert exactly where a ported effect would. Generic parameter list first (no
  editor window), state blob base64'd into `.volt`, reported plugin latency compensated with a
  matching delay on the dry/parallel paths. This stage is where this plan hands over to
  "### 2. VST3 host" below.

**What stays in the browser, deliberately:** automation lanes and mix LFOs (`applyLfos`) keep
running in JS and stream param changes over IPC — moving them native is part of the transport
rewrite, not the mixer. Sample-accurate automation lands when the clock owns the song, not before.
- **Ship point:** stage C — the ASIO path has faders and EQ that do something. `applyMix()` sends
  the mixer state across the same way `nengPattern()` sends notes.

### 0. DECIDED 2026-08-10: it is VST3, and instruments come first
Counted on Mickey's machine: **129 plugins in `C:\Program Files\Common Files\VST3`** — FabFilter
(Pro-Q 3, Pro-L 2, Pro-C 2, Saturn, Timeless), Valhalla, Eventide, A.O.M, Sonible, Waves,
Jup-8000 V — plus a VST2 folder and **no CLAP folder at all**. So the format question is closed:
**VST3 via `vst3-sys`**, and CLAP is not worth a line of code until he owns a CLAP plugin.

**And start with step 5, not step 2.** A VSTi needs no mixer — it takes notes and returns audio,
which is what `trig()` already does. Jup-8000 through ASIO at 5.8 ms is a real result that does not
wait on the largest job on this list. Effects still need the channel strip; instruments never did.

**The first session's task, concretely:** scan the VST3 folder, instantiate one plugin,
`setupProcessing` / `process`, note-on from the clock, audio out through the existing device. No
GUI window — the generic parameter list is an afternoon and unblocks testing everything else.

### 2. VST3 host — BUILT 2026-08-10 (instruments; effects next)

`native/vsthost` is the host: scan, load, instantiate, process. `native/engine/src/vst.rs` makes a
loaded plugin an `AudioWorkletNode`, so **a VST is an ordinary node in the graph** — it connects
where a `GainNode` connects, which is what makes the insert slot in `mixer.rs` work for plugins with
no further plumbing. Notes reach it by `post_message` carrying an absolute time, and the processor
places each one at its own sample offset inside the block it belongs to.

**Measured, not assumed:** Vital and Jup-8000 V both play from VOLT's clock through ASIO4ALL at
**5.80 ms**, with a meter on the plugin's output confirming sound rather than "it didn't crash"
(peak 0.40 / 0.41). Offline too: `vstprobe <name> <out.wav>` renders a chord to a file.

- **`vst3` (coupler-rs), not `vst3-sys`.** The crate this roadmap originally named is not published
  on crates.io. `vst3` 0.3 is, is MIT/Apache, and covers the same interfaces. The Steinberg SDK
  problem is solved the same way it was for ASIO — by not needing it: these bindings reimplement the
  interfaces rather than wrapping the SDK.
- **The VST3 folder has three layouts, and all three are on this machine.** A plain DLL named
  `Thing.vst3`; a bundle `Thing.vst3/Contents/x86_64-win/Thing.vst3`; and vendor subfolders holding
  either. A scanner that knows only the documented bundle form finds about half of them.
- **Scan result: 1347 plugin classes, 31 of them instruments, from 129 files.** Nothing failed to
  load.
- **Every one of the 21 distinct instruments was then loaded and sent a chord.** No crashes.
  Nine make a sound with no further help: **Vital (0.85), Diva (0.76), Twin 2 (0.96), Jup-8000 V
  (0.48), FabFilter One, Newfangled Generate, and the Waves Flow Motion / Element / CODEX.**
  The other twelve — Kontakt, Phase Plant, ACE Bridge, Amigo, TextToSample and the Waves sampled
  instruments (Clavinet, Electric 200/88, Electric Grand 80, Grand Rhapsody, Bass Fingers, Bass
  Slapper) — load, process and output silence, because they want a preset or a sample library
  loaded first. **That is the next piece of host work: `IEditController`**, which is also what
  step 3 (state in the song file) and the generic parameter panel need. One job, three payoffs.
- **Present a real host context, or good plugins crash.** `IComponent::initialize` takes a host
  object and the spec allows null. Passing null cost two of Mickey's synths outright: **u-he Diva
  and Phase Plant crashed with an access violation inside `initialize`**, because they ask the
  context for `IHostApplication` and use the answer unchecked. `host.rs` now presents a minimal one
  (a name, and a factory that politely says no). **Diva went from crash to playing at peak 0.76**;
  Phase Plant went from crash to loading and needing a preset.
- **A plugin must keep its module alive, and the module must never be unloaded.** Two separate
  lessons from the same afternoon. First: a `Plugin` holds an `Arc<Module>`, or the DLL can unload
  while an instance lives — the plugin's own destructor is *in the code being unmapped*. Symptom
  was a process that played correctly, printed its result and then never exited. Second: even done
  right, `FreeLibrary` on a plugin DLL crashes. The library is now deliberately leaked, which costs
  one mapping per plugin file per session and buys not crashing. **And `ExitDll` must not be called
  either** if the DLL stays mapped: doing so during a scan put Diva and Phase Plant straight back to
  crashing, before they had even been reached.
- **A plugin host must end its own process.** After everything is released cleanly, Windows still
  walks every loaded DLL's `DllMain(PROCESS_DETACH)` — and that is where plugins fall over: Jup-8000
  V never returned from it at all (the program hung after printing its results and had to be killed),
  Vital, Diva and Kontakt came back with an access violation. `std::process::exit` runs the same
  teardown and does not help. `volt_vsthost::hard_exit` flushes and calls `TerminateProcess`, which
  is what a host is for. **All five test plugins now exit 0, Jup-8000 in 8 s where it used to hang
  forever.** The desktop app must do the same on close, and anything that needs to be durable has to
  be written to disk before that point.
- **`IEditController` is built too** — created, connected to the component through both
  `IConnectionPoint`s, and handed the component's state through an `IBStream` (`stream.rs`, with
  tests for the seek/short-read/null edges a plugin will find). That is also the object step 3 needs:
  saving a plugin into a `.volt` file is `getState` into the same stream, base64'd.
- **The twelve silent instruments are not a host bug, and this was checked rather than assumed.**
  Phase Plant, Kontakt, Amigo and the Waves sampled instruments report exactly what Vital and Diva
  report — one event input bus, a controller with 165–2608 parameters, stereo out — and produce
  silence anyway. Adding the controller, bus arrangements (`setBusArrangements`) and a running
  transport (`ProcessContext` with tempo and `kPlaying`, advanced every block) changed none of them,
  and all three are correct to have anyway. **They start empty by design**: Phase Plant is modular
  and a new instance has no generators, Kontakt has no library loaded, the Waves instruments want
  their content. Nothing reaches them short of a preset, which means the editor window or a preset
  browser — steps 3 and 5, not a missing call.
- Still to do for effects: an effect is the same code with an audio input bus wired up, dropped into
  `Channel`'s `fx_in`/`fx_out`. **Ship point:** Pro-Q 3 on a channel, heard through ASIO.

### 3. Plugin windows
VST editors draw into a native HWND the host supplies. Tauri can open extra native windows; handing
a plugin a raw window handle from a webview app is fiddly and is where a weekend goes. Until it
works, plugins are still usable through their generic parameter list — do that first, it is an
afternoon and it unblocks testing everything else.

### 4. State, automation, delay compensation
- A plugin's settings are an opaque blob. It has to be base64'd into the `.volt` file and handed
  back on load, or **every song using a plugin opens wrong** — that is rule 3 of the house rules and
  it is not negotiable.
- Parameters into the automation lanes.
- A plugin reporting 512 samples of latency has to be compensated or everything it touches drifts.

### 5. VST instruments
A VSTi takes notes and returns audio — exactly what `trig()` already does. It is a new instrument
type that happens to be a DLL. **Not blocked by the mixer**, so it can jump the queue if what Mickey
wants first is his own synths rather than his own effects. Say which.

### What to do first, concretely
**Port one channel strip and null it against the browser.** Nothing else can be tested honestly
until a note can travel through a fader in Rust and come out matching. `tests/parity.test.js` is
already the tool for grading it.

## VSTs — how we get there, and what actually blocks it (asked 2026-08-10)

Short version: **a browser tab can never host a VST, the desktop build can, and the thing standing
in the way is not the plugin format — it is that VOLT's mixer is still in JavaScript.**

**Why the browser is out, permanently.** A VST is a native DLL. There is no web API that loads one,
there never will be, and no amount of WASM changes it — the plugin expects a real process, a real
audio callback and a real window handle. So this is a `volt-desktop.exe` feature only, and it is
the strongest argument yet for the native engine existing at all.

**What a host actually needs, in the order the work bites:**

1. **A plugin host in Rust.** The honest options:
   - **CLAP via `clack`** — clean, modern, safe Rust bindings, no SDK licence problem. But the
     plugins Mickey owns are almost certainly VST3, and CLAP support is still mostly u-he, Bitwig
     and the newer boutique developers. Right format, wrong library for his folder.
   - **VST3 via `vst3-sys`** — raw COM bindings, unpleasant but real. The Steinberg SDK is not
     redistributable, exactly like the ASIO SDK, and we have already solved that shape of problem
     once (`CPAL_ASIO_DIR`, kept out of the repo, carried in the build env).
   - VST2 is not an option: Steinberg withdrew the licence, and shipping one is not something to
     do on purpose.
   The likely answer is **VST3 first, CLAP alongside it later** — the two APIs are similar enough
   that a `Plugin` trait over both is not fantasy.
2. **A window per plugin.** VST editors draw into a native HWND that the host supplies. Tauri can
   open extra native windows, but handing a plugin a raw window handle from a webview app is real,
   fiddly work and it is where a weekend disappears.
3. **State in the song file.** A plugin's settings are an opaque blob it hands you. That has to be
   base64'd into the `.volt` file and given back on load, or every song that uses a plugin opens
   wrong. Non-negotiable given rule 3 of the house rules.
4. **Automation and delay compensation.** Parameters have to reach the automation lanes, and a
   plugin that reports 512 samples of latency has to be compensated or everything it touches drifts.

**The real blocker, and it is not on that list.** VST *effects* need somewhere to sit — a channel
strip, a send, a master chain. All of that is still Web Audio, in the browser, at ~58 ms. There is
no native mix bus for a plugin to live in. So:

- **VST effects** are gated behind moving the mixer to Rust — EQ, sends, the FX chain, the master
  chain. That is a much bigger job than the transport rewrite and should be planned on its own.
- **VST *instruments* are not.** A VSTi takes notes and returns audio, which is exactly what the
  ported `trig()` already does — a new instrument type that happens to be a DLL, connecting to the
  destination like `sub` and `sampler` do today. **That is reachable without a native mixer**, and
  it is the version worth building first.

**So the order is:** finish the transport (song mode) → **VSTi as an instrument type** → native
mixer → VST effects. Nothing here is blocked on a decision; it is all just work, and the first
useful step is much smaller than the whole feature sounds.

## SCOPE CHANGE — samples through ASIO, 2026-08-10

Mickey, mid-session: *"i dont give a shit about the instruments if needed i want to use only
samples at the moment but i want to be able to render sounds via asio so we have equals to zero
latency you get me?"*

That deletes most of the remaining work. The six-step order below assumed all 14 instruments and
15 effects had to be ported before the native engine was worth using. If the requirement is
**samples out of ASIO**, the only instrument that matters is `sampler` — which is ported and
nulls at **-118 dB**. Steps 5 and 6 are no longer on the critical path.

The path to what he actually wants:

- [x] ASIO device selection — 5.80 ms on ASIO4ALL.
- [x] Clock — 6 tests, no drift.
- [x] `sampler` ported and nulled.
- [x] **Samples reach the engine** — `eng_load_sample` takes interleaved f32 at the device rate,
      keyed by the sample's hash so shared audio is sent once. The webview decodes (it already has
      `decodeAudioData` for every format Chromium knows) and resamples with an
      `OfflineAudioContext`, rather than Rust growing a second decoder and an unmeasured
      resampler. `nengPattern()` sends trim, pad and `Sxx` as params; uploads happen once per
      sample, not per note.
- [x] **Samples play live through ASIO.** `live <device> sample` plays four sampler notes,
      transposed, out of a real-time callback: **5.80 ms on ASIO4ALL, 12.00 ms on the nio.** The
      null test is offline and says nothing about surviving a 256-frame ASIO callback; this does.
- [x] **The click path is tested.** A stubbed `window.__TAURI__` injected *before* the page loads
      (`open({pre})` in `tests/lib.js` — it has to be before, because the build decides once at
      load whether it is inside Tauri and the `const` cannot be reassigned) makes the desktop
      branches live. The palette commands are then run for real and their payloads asserted:
      order, ASIO offered first, **the sample uploaded once rather than per note or per play**,
      keyed by hash, trim as params, `Sxx` as a 0..1 offset, and `srcInst` stripped so Rust can
      deserialise. 29 checks. Suite total **634, 0 failing**.
      - What is still unproven: the real Tauri IPC in the real webview. WebView2 under Tauri
        release refuses `--remote-debugging-port`, so the stub is as close as headless gets. The
        risk that remains is a serde mismatch, not logic.
- [x] **The transport drives it** — "Native engine: follow the transport" in the palette, **off by
      default and remembered**. With it on, Play sends the pattern to the engine as a loop and Stop
      stops it.
      **Additive, not a replacement, and that is deliberate.** `pump()` owns the playhead,
      automation, note-offs, the mixer and recording; swapping it out is a transport rewrite, not
      a flag, and it is the one change that could break Mickey's songs. So with the flag on you
      hear *both* engines, the ASIO one arriving first — useful for hearing the difference and for
      playing to, and honest about being a preview rather than pretending it is the transport.
      Tested for the thing that matters: **with the flag off, Play sends nothing at all**, and Stop
      stops the engine either way so no loop is ever left running out of sight.
- [x] **Reverse (Bxx)** — reversed copies cached on `Ctx` the way `revBuf` caches them in the
      browser, and the trim region mirrors with the audio (`s0' = 1-s1`) or you hear the wrong part
      of the sample backwards. Nulls at **-119.3 dB** plain. **With a trim region: -68.6 dB** —
      passes the bar but 50 dB worse than untrimmed, which at rate 1.0 (no resampling involved)
      means a sub-sample or one-sample offset in the flipped region, not noise. Left as a known
      gap rather than called clean; worth chasing before slice kits lean on it.
- [x] **`sliced` ported (8 of 14)** — chopped breaks: one buffer, a slice list, and the note picks
      the region upward from `sliceBase`. Nulls at **-122.1 dB**. Slices travel as flat
      `slice0s`/`slice0e` params rather than a nested array, so `Job::params` stays a flat map of
      numbers on both sides and the serde contract stays trivial. Attack is a flat 0.4 ms with no
      `fin`: a chop is meant to be abrupt, the transient *is* the sound.
- [ ] **OPEN BUG — an out-of-range note on a slice kit.** Note 99 against a 4-slice kit (index 51)
      should clamp to the last slice. Measured 2026-08-10: the Rust render is **byte-identical** to
      asking for slice 3 directly, so its clamp is right; the browser produces something matching
      **neither slice 3 nor slice 2**. So the browser is the odd one out, and a null test would be
      asserting the wrong behaviour — the case is left OUT of `parity.test.js` with a comment
      rather than passed or fudged. Worth finding: it is what happens when someone plays above a
      kit's range, which is easy to do by accident.
- [x] **The playhead comes back** — `neng:row`, emitted as each row sounds. See step 1 above.
- [x] **Note-offs, and one voice per lane** — a new note cuts the last, an OFF releases it. See
      step 2 above. Also where the engine stopped leaking every node it ever made.
- [x] **ASIO only** — the webview's speakers muted by one gain node, off by default. See step 4.
- [ ] Next: **song mode** — the engine plays one pattern on a loop; a song is a sequence of them
      with a change at the boundary, re-read each row the way `pump()` re-reads `SEQ.songMode`.
      Until then, ASIO-only refuses to engage in song mode rather than playing the wrong thing.

## BUILD ORDER — start here (written 2026-08-10)

Mickey, fairly: *"why cant we start building the fucking thing?"* Nothing is blocking it. The
sequencing above was cautious, not required. This is the order to build the real native VOLT —
each step ends somewhere usable, so it can be stopped between steps but not inside one.

1. ~~**Vendor the patched crate, host-aware.**~~ **DONE 2026-08-10.** `native/vendor/web-audio-api`
   is a fork of 1.7.0 (every edit marked `VOLT FORK`), wired in with `[patch.crates-io]`.
   `all_hosts()` replaces the single `get_host()`, device ids are salted with the host name so the
   same interface under two APIs stays two entries, labels carry the API, and `cpal_device_for_id`
   returns *the host the device was found on* — a cpal `Device` only works with streams built
   through its own host, which is the bug that confined everything to WASAPI. Measured through
   `live`: **ASIO4ALL 5.80 ms, the nio via WASAPI 12.00 ms**, both playing, both chosen from one
   list. `native/asio-host.patch` is kept only as the record of the naive version and why it was
   wrong. Not upstreamed yet — worth doing, and the fork is small enough to rebase.
2. **The clock.** *Started 2026-08-10 — the prerequisite is done and the small version runs.*
   - **`Ctx` is now generic over `BaseAudioContext`.** It was pinned to `OfflineAudioContext`, so
     `trig()` could only be *measured*, never *heard* — `live.rs` kept its own hand-copied kick and
     hat, meaning the thing being played was provably not the thing being null-tested. One `trig()`
     now serves both. Parity re-run after the change: **78 checks, 0 failing.**
   - `live` plays rows off `rowDur() = 60/bpm/lpb` through the real ported instruments, measured at
     **5.80 ms on ASIO4ALL and 12.00 ms through the nio on WASAPI**.
   - **`src/clock.rs` is `pump()`** — 30 ms tick, 160 ms lookahead, rows booked at absolute times
     so nothing is ever played "now". Deliberately free of audio types, so it is testable as
     arithmetic: **6 unit tests**, covering every row scheduled exactly once, a 200 ms stalled tick
     losing nothing, `Txx` as 192nds of a row, and **ten minutes at 175 bpm without drift**.
   - Still to do: re-reading song-vs-pattern mode each row (needs a song structure to read from),
     and wiring `Txx` through from pattern data — the clock computes it, nothing supplies it yet.
3. ~~**Vertical slice.**~~ **DONE 2026-08-10.** `live <device>` plays a 16-row pattern through the
   clock and the ported `trig()`, out of any ASIO or WASAPI device: **32 rows, 24 voices, 5.80 ms
   on ASIO4ALL and 12.00 ms on the nio.** Rust owns the time and makes the sound. What it does not
   yet have is *your* song — the pattern is hardcoded, which is step 4.
4. **IPC.** *Started 2026-08-10 — the wire exists and the desktop app links the engine.*
   - `native/desktop/src/engine.rs`: `eng_devices` / `eng_open` / `eng_play` / `eng_stop`. The
     `AudioContext` lives on its own thread behind a channel, because a live audio callback and
     Tauri's command pool must not touch the same graph. Opening a device replaces the previous
     one — ASIO drivers take the hardware exclusively.
   - In the palette (desktop only): **Native engine: choose output device / play current pattern /
     stop.** `nengPattern()` converts the current pattern to the engine's shape and **skips
     instruments that are not ported yet rather than dropping them silently.**
   - **The dependency conflict worth knowing about.** cpal 0.18.1 asks for
     `windows-core ">=0.61, <=0.62"` but for `windows "0.61"`, and runs windows-core's
     `#[implement]` macro over `windows`' interfaces. Cargo takes the top of the range while
     Tauri's tao/wry pin `windows` at 0.61, so the two halves disagree and WASAPI will not
     compile. The engine crate alone never sees this. Pinned in `native/desktop/Cargo.toml`.
   - `tests/nengine.test.js` covers the JS half — **16 checks**: velocity scales 0..127 → 0..1,
     note-OFF never crosses as a note, `Txx` survives as 192nds, parameters travel with the note,
     unported instruments are dropped *and reported*, and the desktop-only palette entries stay
     hidden in a browser tab. Suite total now **612 checks, 0 failing.**
   - Still to do: the playhead coming *back* (needs a Tauri event channel), song mode, automation,
     the mixer, and deciding whether Rust should own song state rather than receiving a copy.
   - **Not yet verified: the click path.** The Rust commands and the JS conversion are each tested,
     but nobody has driven the palette entries in the running app — a webview menu cannot be
     exercised headlessly. That is the one thing to check by hand first.
5. **Sampler + Slices**, then reese / mutant / supersaw / psybass / W0RM-303, then the effects —
   each one nulled against the browser by `tests/parity.test.js` as it lands.
   - **`sampler` is ported (6 of 14), 2026-08-10** — playback rate from root note, trim region,
     Sxx offset, pad, fade in/out. The sample rides on `Ctx::sample` rather than in `Job`, because
     a note crosses IPC and a sample is megabytes that must be decoded once and shared.
     `Job::opt` joins the strict `p()` for the parameters the browser itself treats as optional.
     Two details that look like rounding and are not: the 0.4 ms attack floor (a ramp to a
     zero-length target clicks), and `dur / rt` for the fade-out, since duration is in buffer
     seconds but scheduling is in played seconds — only wrong on transposed notes.
   - **Nulled, 3 cases.** Neither side loads a WAV — both generate the same buffer from the same
     seeded PRNG, so what is compared is the playback and not two decoders. Noise rather than a
     tone on purpose: a tone is periodic, so an offset or rate error lands on a similar-looking
     waveform and hides.

     | case | residue |
     |---|---|
     | at root note, no resampling | **-118.6 dB** |
     | an octave up, rate exactly 2 | **-119.3 dB** |
     | trimmed + faded, rate 1.498 | **-63 dB** |

     The first two are effectively bit-identical. The third is 55 dB worse and that is a **real
     difference, not noise**: rate 2.0 is integer decimation, 1.498 needs interpolation, and the
     two engines interpolate differently — the same class of gap as the band-limited oscillators.
     Inaudible, but it means a fractionally-transposed sample is *not* sample-exact, and anything
     built on top of that (slice kits especially) should be judged against -63 dB, not -118.
   - Reverse (Bxx) is not ported: it needs the reversed-buffer cache.
6. **CLAP**, then VST3.

**Tooling, done 2026-08-10:** `tools/volt-mcp.js`, registered as the `volt` MCP server — Mickey's
*"full api mcp to claude so we can tshoot and vibecode it"*. Build, test, list audio devices, play
through one and measure it, search his Everything index, grep into `beta2.html` without loading
716 KB. Health-checked ✔ Connected. See CLAUDE.md.

Steps 1–3 are where the payoff is: after them there is an app that makes sound natively at a
latency Mickey picks. Steps 4–5 are the long tail that makes it *his* songs rather than a demo.

Honest sizing: 1 is a session, 2–3 a few, 5 is the bulk of the remaining months. Not one sitting —
but every step ships something.

## Going native — decided 2026-08-09, measured not guessed

Mickey wants a full DAW: ASIO latency and VST hosting. Both need the audio out of the webview.
The numbers that settled it, from his machine (`native/probe`):

| path | output latency |
|---|---|
| Web Audio, what VOLT uses today | **58 ms** |
| Native WASAPI, through the engine | 12 ms |
| **Native ASIO, best driver on this box** | **5.8 ms** |

Ten times better. There is no browser-side fix for this — Web Audio owns the output device.

> **Corrected 2026-08-09 (evening).** This table first read "Native ASIO 1.45 ms — forty times
> better", and that was wrong. 1.45 ms was `probe` printing a *callback period* (64 frames ÷
> 44.1 kHz), not an output latency, so it was never comparable to the browser's 58 ms. Re-measured
> with the same metric on both sides, the four ASIO drivers on this machine come in at 5.8, 17.3,
> 17.3 and 40.3 ms — two of them worse than WASAPI, one worse than the browser — and none of them
> honours the buffer size the engine asks for. `native/README.md` has the full table and why.
> The case for going native survives the correction; the headline number did not.

- [x] **Rust toolchain + latency probe** — 1.97.1 installed, `native/probe` builds with ASIO.
      SDK at `C:\Users\Mickey\sdk\ASIOSDK`, outside the repo (licence forbids redistribution).
- [x] **Tauri desktop shell** — `native/desktop`, loads `beta2.html` unchanged, 7.6 MB binary.
      Buys a real origin (folder pickers, AudioWorklet) and Chromium. **Does not** buy latency.
- [ ] **ASIO host selection** — proven to work, deliberately not landed yet. `web-audio-api`'s
      `get_host()` never asks for the ASIO host, so the `cpal-asio` feature compiles the backend in
      and then never uses it. A patched copy of the crate reaches all four drivers (5.8 ms at
      best), but the fix as written makes ASIO the *only* host and hides WASAPI, and ASIO4ALL
      leaves a zombie process on shutdown. The right version is a host-aware device picker,
      upstreamed rather than forked. Worth ~6 ms; not worth blocking the port. *Deferred.*
- [~] **Desktop build should use the real filesystem** — started 2026-08-10. Songs now mirror to
      `Documents\VOLT` as real `.volt` files (atomic write, browser `.json` exports listed
      alongside), changeable from the command palette. **Still a mirror, not the source of
      truth**: loading goes through the Projects panel and localStorage, and samples stay in OPFS.
      Finishing it means making the folder authoritative on load. See `native/README.md`.
      Mickey's words: "we need it to access direct mothafkn storage" — an app with its own
      storage island is backwards.
- [x] **Sample browser, backed by Everything (voidtools)** — **done in the desktop build,
      2026-08-10.** Mickey's priority: *"i want to make sure that samples are working 1stly,
      everything voidtools support and all of that, after that we'll go to the generators."*
      The panel itself already existed in the web build; what the desktop adds is a transport
      (`ev_get`) that reaches Everything's HTTP server from Rust, so there is **no CORS header to
      set, no bridge extension and no Local Network prompt** — it works on first launch. Verified
      by `cargo test` against the live server: search returns paths, a WAV comes back as bytes.
      The paragraphs below are the investigation that got there, kept because the dead ends are
      worth not repeating.
      - **This is desktop-only, and that is the point worth arguing about.** A browser tab cannot
        open a named pipe, and Everything's HTTP server sends no CORS headers, so `beta2.html` in
        Firefox can never have this. Building it makes the Tauri app the *primary* target and the
        tab the fallback — the first feature to split them. Worth doing anyway, but it is a fork
        in the road, not just a feature.
      - Measured on Mickey's machine 2026-08-09: **Everything 1.5a is running** (plus a 1.4
        service), from `C:\Program Files\Everything 1.5a`. The named pipe
        `\\.\pipe\Everything IPC (1.5a)` **connects without elevation** — tested. The SDK's
        window-message IPC does *not* work here: `FindWindowW` finds none of
        `EVERYTHING_TASKBAR_NOTIFICATION` / `EVERYTHING` / `EVERYTHING_1.5a`, because the running
        instance is `-isrunas`. So the pipe (or `es.exe`) is the route, not the classic SDK DLL.
      - Three ways in, pick one before writing code: **`es.exe`** (official CLI, one small
        download, no config, works with 1.4 and 1.5 — most robust); **the named pipe** (nothing to
        install, but the 1.5 wire format has to be got right); **Everything's HTTP server** (returns
        JSON, but Mickey has to enable it in Options and it opens a local port). Recommend `es.exe`.
      - Sample *previewing* wants the Rust engine to have a sampler, which it does not yet — see
        below. Browsing and importing can land first, with the browser doing playback.
- [~] **Rust audio engine** — started 2026-08-09, and the wall turned out to be shorter than it
      looked. The 10–16 week figure assumed writing 29 units of DSP from scratch. But VOLT's
      instruments were never hand-written DSP — they're Web Audio node graphs, and
      [`web-audio-api`](https://crates.io/crates/web-audio-api) is that same API, spec compliant,
      in Rust (v1.7, released the day before we started). So each instrument is a *transliteration*
      of the JS that already exists rather than a rewrite, and ASIO comes free underneath it
      because the crate's backend is the same cpal the probe measured.
      - `native/engine` builds; **kick, snare, hat, clap and sub are ported** — 5 of 14.
      - `tests/parity.test.js` renders the same note through both engines and subtracts them.
        Where the spec pins behaviour down they agree from -85 dB to **bit-identical**: gain
        envelopes, biquads, buffer sources, the waveshaper, the seeded noise.
      - Sample-exact parity with Chrome is **not achievable**, and it's Chrome's fault, not ours —
        two of its AudioParam behaviours don't follow the spec (see `native/README.md`). Firefox
        differs again, so there is no single "the browser" to null against. The suite grades on
        RMS and lists the one real remaining gap with the number it measures.
      - **Band-limited saw and square — done.** They were the prerequisite for the melodic
        instruments and they were badly wrong: -15 dB against Chrome and 1.18x too loud, because
        the crate's built-ins are naive and Chrome normalises after band-limiting. Rebuilt from
        the spec's Fourier series as per-octave tables: saw -35.7 dB, square -38.3/-43.9 dB.
        The triangle is deliberately left on the built-in — a table makes it worse — and saw at
        1760 Hz (-21.3 dB) is the worst case left, where Chrome crossfades tables and we don't.
      - **Next is the sampler, not the synths** — reprioritised 2026-08-09 on Mickey's call.
        `sampler` and `sliced` are the two instruments he actually leans on, and the engine has
        neither; all five ported so far are synthesized. `AudioBufferSourceNode` is in the crate,
        so it transliterates like the rest, and it brings `Bxx` (reverse), `Sxx` (offset),
        `playbackRate` and slice regions with it. Then reese, mutant, supersaw, psybass and
        W0RM-303 — what the saw and square work was for — then the effects, then the clock and IPC.
- [ ] **CLAP hosting** (MIT, good Rust support) then VST3 (needs Steinberg's SDK, rougher
      bindings). Trivial once Rust owns the audio callback. Mickey linked
      github.com/steinbergmedia/vst3sdk.
- [ ] **Canvas grid** — the smoothness ceiling, and it pays off in the browser too.

Cheaper thing worth doing first: **plugins on bounce/freeze only**. Rust hosts the plugin offline
and prints it, no engine rewrite, no clock-sync problem. 2–3 weeks for most of the value.

## Now — storage that works in Firefox

The one real gap. Firefox does **not** implement the File System Access API, so the ⚙ Settings
folder pickers (`showDirectoryPicker`) simply don't exist there — no flag, no about:config
switch, no hack. Mozilla has objected to the directory part on security grounds. So instead of
chasing it, get the same *result* a different way:

- [x] **OPFS storage backend** — swap the localStorage fallback for the Origin Private File
      System (`navigator.storage.getDirectory()`). **Verified 2026-08-04** with
      `tools/opfs-probe.html`: works in Firefox 153 AND Chrome, even on `file:///` despite the
      opaque origin, 10 GB quota, 24 MB written+verified, survives a reload. Real files,
      persists across sessions, quota in the hundreds of MB instead of localStorage's ~5 MB.
      Fixes "samples too big for autosave" for good. Invisible in Explorer — that's the trade.
- [ ] **Import a folder** — `<input webkitdirectory>` and folder drag-and-drop both work in
      Firefox. One-shot read (no live sync), but it bulk-loads a sample library in one go.
- [ ] **Export everything** — one button, whole library + projects as a `.zip` download. This is
      how files get *out* of Firefox and onto disk.
- [ ] Settings ⚙ explains which mode you're in per browser, instead of hiding the pickers.
- [x] **Projects panel** — done 2026-08-09. A foldable section at the top of the left rail (not a
      floating panel) that answers "what is this browser holding?": every project open or closed,
      with size and when it last saved; the idea count; the OPFS sample store with how much of it
      nothing points at any more (one click reclaims it); and localStorage pressure against the
      ~5 MB ceiling. Redraws itself on every autosave, costs nothing while folded, and a closed
      project reopens as a new tab — never in place of the one you're in. Toolbar button or
      **Alt+Z** (the only Alt letter left). `tests/home.test.js`.

> Note: test this on the hosted page or a local server, not `file:///` — storage APIs behave
> oddly on file origins, which is also why some things differ between your two test setups.

## Next — the ideas already designed but not built

- [ ] **FX command picker** — typing a wrong letter in the fx column currently does nothing at
      all. A type-ahead list (letter + plain-English name + hint), triggered by Enter, `?`, or
      any unrecognised letter, handing off to the existing value slider. Discovery for the one
      part of a tracker nobody can guess.
- [x] **Armed transport** — Space plays the armed mode, `\` flips Song ⇄ Pattern live.
- [ ] **Split samples out of the song file** — right now they're base64 inside the JSON, so
      projects are megabytes and every autosave rewrites the whole blob. Content-addressed
      sample storage makes projects small, saves fast, and makes any cloud sync realistic.
      *Prerequisite for anything involving Dropbox/Drive.*

## Agreed 2026-08-04 — Mickey's batch

- [x] **Stash the rows/beat with an idea** — done 2026-08-04. `captureIdea` already saved `lpb`;
      nothing used it. `ideaRescale()` now stretches the phrase onto the host song's grid on
      load, carrying sub-row positions as Txx. Ideas with no `lpb` are assumed to be 4.
      Proved by render: 0.0000 s drift. `tests/vault.test.js`.
- [x] **Swing / groove** — done 2026-08-04. Song-wide Swing % + 1/8 or 1/16 next to BPM, with a
      per-channel override in the Track panel (double-click = back to global). Applied at
      schedule time, so the grid still reads straight and it can be dialled while a loop plays.
      Measured: 0.251/0.251 → 0.375/0.128 with the downbeats unmoved. `tests/swing.test.js`.
- [x] **Spectrum analyser, per channel** — done 2026-08-04. Canvas above the Track panel sliders,
      follows the cursor channel (master in master view), log frequency axis, peak-hold, and
      click-to-freeze an amber reference curve for A/B-ing an EQ move. 0.058 ms a frame, drawn at
      30 fps, skipped entirely when the rail is hidden. `tests/spectrum.test.js`.
- [x] **Song structure section** — done 2026-08-04. Named ranges (`song.sections` = `{a,b,name,
      hue}`) drawn as coloured bars **directly over the chips they cover**, one bar per visual row
      when the strip wraps. Drag either end to resize (snaps to whole slots, clamps at the next
      section). Ctrl+Shift+M adds, Ctrl+←/→ jumps — works while playing. Ranges survive reorder
      and delete by keeping their biggest contiguous run. `tests/sections.test.js`.
- [ ] **Genre starter beats** — baked-in patterns to start from: drill & bass, half-time,
      breakbeat, techno, psy. Same mechanism as the existing kits.

- [x] **Arrangement editing** — done 2026-08-05. Song length + running clock on the strip and per
      section; multi-select slots (Shift/Ctrl+click, Ctrl+drag lasso) with Delete / Ctrl+X C V D;
      section bars now sit flush on top of the chips they cover.

- [x] **Disabled notes** — done 2026-08-07. `Alt+N` switches notes off without deleting them
      (cell `d` flag): struck through in the grid, hollow in the piano roll, skipped by the
      scheduler so they're silent in playback and in exports. Treated as empty rather than a
      note-off, so the previous note rings through. `tests/disable.test.js`.
- [x] **Bounce a channel** — done 2026-08-07. `Alt+B` renders the cursor channel (chain, sidechain
      ducking, swing) over a chosen scope — selection / pattern / section / whole mix — into a new
      sampler instrument, and writes the WAV beside the project (or downloads it). Master FX are
      excluded from single-channel bounces. `tests/bounce.test.js`.
- [x] **Freeze** — done 2026-08-08. `Alt+Shift+R` bounces the track, disables its notes, drops one
      hit at row 0 and flattens the channel strip + parks its automation lanes so nothing is
      processed twice; level is matched by measuring the result. Same key thaws it, restoring
      notes, mixer and lanes. Per pattern. `tests/bounce.test.js`.

- [x] **GLTCH-9 glitch percussion** — done 2026-08-08. A note is a ratchet of grains (hits / gap /
      accel), four sources (noise, FM metal, pulse, sine zap), sweep, fall, crush, resonant
      bandpass zap and chaos, with 8 presets. Deterministic per note so renders match playback —
      which also meant seeding the shared noise buffer, fixing a long-standing mismatch for hats,
      snares and claps. `tests/glitch.test.js`.

- [x] **PULTEQ** — done 2026-08-08. Pultec-style low boost + cut (the cut is a DIP above the
      boost, not a second shelf, or they cancel) plus a wide air lift. +6.7 dB at 30 Hz and
      -7.3 dB at 120 with both lows at 8.
- [x] **Limiter L∞** — done 2026-08-08, simplified to **two knobs** (LOUD, CEIL) after the first
      version shipped five nobody could answer. A true L2-style look-ahead brickwall (ScriptProcessor,
      since AudioWorklet can't load a module from `file://`): threshold as drive, ceiling, release,
      ARC, GR metering. Measured 0.00 dB over at every ceiling. Neutralises VOLT's own safety
      compressor while active, which was silently adding ~1.3 dB of makeup to everything.
- [x] **Mastering tools** — done 2026-08-08. Soft Clip FX (drive / knee / ceiling / mix, with the
      drive gain compensated) and a master analysis block: K-weighted LUFS, peak with clip
      warning, mono correlation, and eight band meters with peak hold. `tests/master.test.js`.

- [x] **Automation lane tools** — done 2026-08-09. Ctrl+wheel zoom / fit (Ctrl+right-click),
      a shape generator (sine / tri / saw / ramp / square / repeatable random, cycles, phase,
      low, high) and level scaling (multiply / centre / offset / tension) with live preview.
      Both honour a selected range. `tests/autolane.test.js`.
- [x] **GLTCH-9 retrigger CPU** — fixed 2026-08-09. A 14-hit burst built 46 audio nodes and an
      Rxx16 fired sixteen of them into one row (736 nodes / 86 ms). Sources are now shared across
      grains: **7 nodes whatever the hit count**, same as a kick.

## Editing / workflow

- ~~Interpolate between two values~~ — **rejected 2026-08-04.** Mickey: the automation lanes
      already do this, and drawing a curve beats typing hex digits. Don't re-suggest it.
      General lesson: he wants *less typing*, not faster typing.
- [ ] **Humanize** — nudge timing (Txx) and velocity by a chosen amount. Instant "not a robot".
- [ ] **Scale lock** — pick a key/scale; wrong notes are dimmed in the piano roll, and optionally
      snapped in the grid. Huge for anyone who doesn't know theory.
- [ ] **Chord tool** — type a root, get maj/min/7/9 stamped across poly lanes (the arp already
      knows how to build these).
- [ ] **Undo history browser** — a list you can click back through, instead of 40× Ctrl+Z.
- [ ] **A/B compare** — stash the song, try something wild, flip between the two.
- [ ] **Per-pattern tempo** — tempo change on a pattern, for half-time drops.

## Sound

- [ ] **Send buses** — one reverb and one delay everything can be sent to, instead of an
      instance per track. Cheaper and it's how records are actually mixed.
- [x] **Sidechain ducking** — done 2026-08-04. Was hardcoded to the built-in kick synth; now
      each track picks its own trigger track (Track panel → “Duck←”) with a release control,
      and a muted trigger still pumps. Measured in `tests/duck.test.js`.
- [ ] **Compressor insert** — there's a limiter on the master but no per-track glue.
- [ ] **Sample recorder** — record from the mic straight into an instrument slot.

## Import / export

- [ ] **MIDI export** — get a pattern out to a real DAW.
- [ ] **MIDI import** — drop a `.mid` on the grid.
- [ ] **Stem export** — render each track to its own WAV in one pass.

## Maybe / thinking about it

- [ ] **Cloud project storage** — Dropbox is the easiest OAuth from a static page; Google Drive
      needs the `drive.file` scope to avoid heavy app verification. Blocked on sample-splitting
      above, and OAuth redirects don't work from `file://`, which breaks the local test loop. [?]
- [ ] **Pattern queue / live launch** — trigger patterns Ableton-style for playing live. [?]
- [ ] **Share a song by link** — compressed song in the URL, or a tiny paste service. [?]

## Done

- [x] Arpeggiator with live playback, 7th/9th colouring, Txx micro-timing (2026-07-30)
- [x] Cursor stays visible while playing (Home/End/PgUp/PgDn) (2026-07-30)
- [x] Editable rows-per-beat with lossless rescale, Shift+`[` / Shift+`]` (2026-07-31)
- [x] Grid performance: 8.7 fps → 43 fps on a 32×128 pattern (2026-07-31)
- [x] 3 new themes (mono / midnight / paper), Alt+L rail hide, Alt+H column focus (2026-07-31)
- [x] Test suite in the repo — `node tests/run.js` (2026-07-31)
- [x] Armed transport mode + `\` live Song/Pattern flip (2026-08-04)
