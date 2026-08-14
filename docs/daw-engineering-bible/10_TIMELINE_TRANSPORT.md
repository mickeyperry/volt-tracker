# Timeline and Transport

## Canonical state

The engine owns a signed 64-bit (or wider checked) absolute **sample position** at the project sample rate. Musical position, seconds, bars/beats, and UI coordinates are derived projections. Never accumulate floating-point beat increments per sample/block.

An immutable tempo map contains ordered segments with tempo/ramp law and time-signature markers. Conversion routines map sample <-> musical position deterministically with specified rounding. Time signatures affect bar notation/grid, not audio clock rate.

## RT transport snapshot

Each quantum receives:

- transport state/generation (`stopped`, `starting`, `playing`, `recording`, `stopping`);
- absolute project sample start and frame count;
- possibly split subranges for tempo, loop, punch, or discontinuity boundaries;
- musical position/tempo/time signature at boundaries and plugin process context;
- loop, punch, preroll/count-in state;
- discontinuity/seek flags.

Control prepares transitions; RT applies them at an exact sample boundary. UI state is an observation.

## Boundary splitting

If a quantum crosses a loop end, tempo-segment boundary, seek target, punch edge, or scheduled transport change, split it into bounded subranges so clips/events/automation observe exact positions. Precompute upcoming boundaries and cap pathological maps; compilation rejects or normalizes more boundaries than the prepared capacity.

## Operations

- **Play**: publish start position and prepared stream/event horizon; start at exact boundary with de-click policy.
- **Stop**: define whether tails continue, instruments receive all-notes-off, and position remains/returns. File/plugin work stays off-RT.
- **Seek**: increment timeline generation, invalidate old stream/lookahead/event work, reset or preserve plugin/DSP state by explicit policy, preroll hidden state if correctness requires it.
- **Loop**: wrap project sample mapping exactly; prefetch both sides. Sustain/note behavior across wrap is explicit.
- **Record**: timestamp capture from device-clock/sample counter; compensate known input latency only under a documented placement model.
- **Punch**: capture may run through a wider safety region, while committed take boundaries are sample accurate.
- **Preroll**: run graph/streams before audible or record boundary to warm latency, plugin state, and tempo-dependent effects.
- **Count-in**: metronome/event scheduling occurs before record start without redefining project sample truth.

## Automation

Automation is compiled into per-parameter, sample-domain segments/events for upcoming ranges. Evaluation is sample accurate where processor API supports it; otherwise host-defined block/sub-block behavior is documented. Tempo-relative points are converted using the same tempo-map generation as playback. Edits invalidate only affected future ranges.

## Determinism and discontinuities

Define rounding (`floor`, nearest, tie behavior) once for sample/musical conversion. Use checked arithmetic for long projects. A seek/discontinuity propagates to plugins, clip resamplers, automation cursors, MIDI generators, delay/reverb reset/preroll policy, and cache keys.

## Tests

Property-test round trips around tempo ramps, signatures, negative preroll positions, long sessions, and exact boundaries. Render loops/punch/automation and inspect sample onsets, not only state. Test different device quantum sizes yield equivalent output within declared DSP tolerance.

## References

- Steinberg, [VST3 ProcessContext](https://steinbergmedia.github.io/vst3_doc/vstsdk/structSteinberg_1_1Vst_1_1ProcessContext.html).
- CLAP, [transport event specification](https://github.com/free-audio/clap/blob/main/include/clap/events.h).

