# MIDI and Event System

## Unified event representation

All engine events carry:

```
absolute_sample_time | block_sample_offset | stable_sequence
type | route/source | target | channel/note/parameter | payload
```

Absolute sample time is canonical in queues/caches. At quantum preparation, events in `[start, end)` receive integer offset `0..frames-1`. `stable_sequence` resolves equal-time ordering reproducibly.

Supported families include note on/off (including note-on velocity zero normalization policy), poly/channel pressure, CC, program change, pitch bend, per-note expression, SysEx where supported/bounded, plugin-native note events, parameter automation/modulation, transport/safety events, and MIDI clock/MTC where in scope.

## Producers and timing

| Producer | Timestamp source | Path |
|---|---|---|
| Project clips | timeline samples/tempo map | control compiles future slices |
| Automation | timeline samples | compiled segments/events |
| Hardware MIDI | backend timestamp mapped to audio device clock | bounded input queue |
| UI keyboard/controller | monotonic receipt mapped to next valid audio time | bounded command queue; latency disclosed |
| Plugin output | sample offset in current process call | downstream graph event edge |

Never time musical events from UI animation frames. Calibrate MIDI/input clock-to-audio clock mapping, retain timestamp uncertainty metrics, and handle drift/discontinuities.

## Ordering

Sort by sample time then stable sequence, with explicit same-sample safety rules. Preserve authoring order unless a rule is necessary. A generated note-off/all-notes-off must not be dropped behind low-priority CC during overflow. Define whether note-off precedes a retriggering note-on for the same identity; test it.

Merging routes is a fan-in dependency. Plugin-produced events are available only after that plugin processes, so downstream instruments/effects depend on it. Ordinary precompiled clip events do not serialize audio nodes beyond their target.

## Buffers and overflow

Use fixed-capacity contiguous per-node event spans from the event pool; no linked allocation. Compiler estimates/project-validates static density, while live/plugin input uses a configured ceiling. On overflow:

1. preserve note-off, all-notes-off, transport safety, and essential state;
2. coalesce replaceable continuous controllers/parameters by target and time range where semantics allow;
3. drop lowest-priority events deterministically and emit a counter;
4. never write past capacity or grow on RT.

## MIDI clock

Incoming MIDI clock is asynchronous and jittery. A clock-follow subsystem outside the hot DSP path filters/estimates phase and publishes bounded tempo/transport corrections; it must not block callback. Outgoing clock events are scheduled from sample time with backend timestamp support where available. Document latency/jitter limits; MIDI clock is not sample clock.

## Note safety

Track active-note identity including note/channel/port and note ID where supported. Stop, seek, loop, routing removal, plugin bypass, and device disconnect apply explicit flush/retrigger policy. Do not rely on UI to send cleanup. Sustain and sostenuto state are part of routing state.

## Tests

- Same-sample ordering, quantum-boundary offsets, dense chords/CC, long timestamps, tempo ramps.
- Loop/seek/stop stuck-note prevention and sustain behavior.
- Plugin event fan-in/out and format translation round trips.
- Hardware timestamp jitter/drift simulation.
- Output equivalence across block sizes where processor semantics permit.

## References

- MIDI Association, [MIDI 1.0 messages](https://midi.org/summary-of-midi-1-0-messages).
- Steinberg, [VST3 Events](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical%2BDocumentation/Hosting/Processing.html).
- CLAP, [events specification](https://github.com/free-audio/clap/blob/main/include/clap/events.h).

