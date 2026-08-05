# VOLT — working notes for whoever picks this up

VOLT is a tracker / DAW that runs entirely in a browser tab. One HTML file, no build step, no
dependencies at runtime. Mickey uses it to actually make music, so **breaking his songs is the
one unforgivable failure** — everything below is aimed at that.

Live: https://mickeyperry.github.io/volt-tracker/beta.html

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
| `beta.html` | **where all work happens.** ~600 KB, one big inline `<script>`. |
| `volt.html` | the stable build. Don't touch unless promoting a release. |
| `tests/` | `node tests/run.js` — 119 checks, must stay green. |
| `tools/opfs-probe.html` | standalone storage probe (open in a browser, reports what works). |
| `ROADMAP.md` | **the feature list and progress.** Read it when he asks "what's next". Tick items off there. |

Beta and stable share an origin but **never share data**: line 5 of `beta.html` sets
`window.VOLT_NS="voltbeta"` and rewrites every `volt.*` localStorage key to `voltbeta.*`.
`VNS` in the main script keys OPFS and IndexedDB off the same thing. If you add persistent
storage, namespace it — a bug in beta must not be able to reach his real songs.

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
- Editing the file with CLI tools invalidates the Edit tool's state — Read a few lines first.
- `pointerup` blurs buttons and ranges globally. **Never remove it** — hotkeys die without it,
  because the grid's `mousedown` preventDefault blocks refocus.

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

Only when asked. `beta.html` is tracked on `main` and GitHub Pages serves it directly — commit
and push is the whole deploy. Run the full suite first. The `dev` branch is stale; ignore it.
