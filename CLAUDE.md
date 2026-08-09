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
| `tests/` | `node tests/run.js` — 504 checks against `beta2.html`, must stay green. |
| `tests/profile.js` | `node tests/profile.js` — not a test. Prints where the time actually goes (switch, undo, autosave, frame gaps) so a speed claim can be argued from numbers. |
| `tools/opfs-probe.html` | standalone storage probe (open in a browser, reports what works). |
| `ROADMAP.md` | **the feature list and progress.** Read it when he asks "what's next". Tick items off there. |

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
- Editing the file with CLI tools invalidates the Edit tool's state — Read a few lines first.
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
