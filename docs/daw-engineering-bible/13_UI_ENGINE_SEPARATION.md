# UI and Engine Separation

## Ownership rule

The UI never owns sample position, transport truth, active graph, automation timing, audio buffers, plugin process state, or recording placement. It submits intent and renders observations. Closing, blocking, or rebuilding the UI cannot stop audio.

## UI-to-engine commands

Commands are immutable, bounded, versioned messages with stable IDs and an explicit timing policy:

- structural edit: apply to project model, compile prepared graph, publish later;
- transport command: target exact sample or defined next safe boundary;
- sample-accurate parameter/note: timestamp into event system;
- latest-value control: atomic/coalescible only when intermediate values have no semantic value;
- non-RT request: save, scan, waveform, plugin GUI, cache routed directly to owning service.

The UI receives accepted/rejected/pending state. A queue-full structural edit is not silently lost. Do not pass UI object pointers/closures into engine code.

## Engine-to-UI telemetry

Publish compact snapshots/rings containing generation and timestamp:

- transport sample position/state and tempo projection;
- meter peaks/RMS, clip indicators;
- callback/deadline/underrun summaries;
- per-node/plugin performance summaries;
- stream starvation/pool high-water/status;
- recording progress and command acknowledgments.

Telemetry is lossy by design. UI reads newest complete snapshot at its frame rate and interpolates playhead from a timestamp only for drawing. It never writes interpolation back as transport truth.

## Meters, waveforms, visualizers

RT computes only bounded primitives already needed or cheap accumulators into fixed slots. UI/background workers perform smoothing, history, FFT visualization, waveform pyramid generation, image building, and formatting. If UI misses frames, drop visual frames—not audio. Analyzer DSP that is musically part of a plugin graph follows normal node rules; decorative analysis is optional background work.

## Plugin GUI bridge

Platform main thread owns plugin windows/views and event pumping. Parameter gestures use the same command/event model as native UI. Editor resize/repaint/timer work never runs on callback. Hide or destroy follows plugin compatibility policy from `07_PLUGIN_ARCHITECTURE.md`.

## Snapshot consistency

Snapshots are immutable, double/triple-buffered or queue-published with generation checks. UI must tolerate skipped generations and project/engine lag during graph compilation. Avoid seqlock read loops over huge snapshots and avoid UI-held locks needed by engine/control publication.

## Failure behavior

- UI hang: audio continues; telemetry overwrites old data.
- Engine command rejected: UI rolls back/presents error using command ID.
- Graph compile slow/fails: old graph continues and UI shows pending/error revision.
- Plugin editor crash/hang: process/sandbox policy applies; no callback wait.
- Meter overload: reduce telemetry rate/detail automatically outside RT.

## Tests

Block UI thread for seconds during playback/record; resize continuously; open many plugin GUIs; saturate telemetry; issue rapid structural edits; close/reopen panels. Assert callback statistics and output remain within baseline tolerance and command state converges.

