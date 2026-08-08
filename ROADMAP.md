# VOLT roadmap

The running list of what's next. Tick things off as they land; add anything you think of.
Everything here is a suggestion until you say otherwise — nothing is committed to.

**New here?** Read `CLAUDE.md` first — house rules, architecture, and the traps.

**How to use this:** open it any time (it's in the repo, so it's also readable on GitHub from
your phone). Or just ask Claude *"what's on the VOLT roadmap?"* and it'll read this file, and
update the ticks when it finishes something.

Status: `[ ]` not started · `[~]` in progress · `[x]` done · `[?]` needs a decision from Mickey

---

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
