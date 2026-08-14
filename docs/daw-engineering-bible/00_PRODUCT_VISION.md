# Product Vision and Non-Negotiable Outcomes

## Goal

Build an interactive professional workstation that remains correct and responsive under low-buffer, plugin-heavy, highly routed projects. The engine is a deterministic deadline system, not a collection of track loops. Optimize the worst callback and graph critical path before average throughput.

## User-visible success

- Clean playback and monitoring at 48 kHz / 64 and 128 frames on supported hardware.
- Sample-accurate clips, MIDI, automation, loops, punch boundaries, and PDC.
- Project size scales with independent graph work across performance cores.
- UI stalls, waveform work, autosave, scans, and storage latency cannot stall audio.
- Repeatable offline output where plugin behavior permits; differences are disclosed and testable.
- Device loss, stream starvation, plugin fault, overload, and graph-edit behavior are explicit rather than undefined.
- Existing projects and audio assets remain recoverable. Serialization has versioned migration and atomic persistence.

## Architecture stance

The project model is editable and rich. The live engine model is compact, immutable, and precompiled. A control-domain compiler translates the former into the latter, validates routing, resolves latency, reserves memory, and publishes a generation for a quantum-boundary swap.

```
project intent -> validate/compile -> prepared graph -> atomic publication
                                                   -> RT quantum execution
device input -> RT graph -> device output          -> bounded telemetry
disk/decode -> read-ahead FIFOs                     -> UI snapshots
```

Tracks are presentation and editing concepts. Scheduling units are dependency graph nodes; **one track is not one thread**.

## Product modes and tradeoffs

| Mode | Primary constraint | Permitted optimization |
|---|---|---|
| Live input/record monitoring | Minimum bounded latency | No anticipative processing on input-dependent descendants |
| Playback/mixing | Deadline reliability | Lookahead processing for causally independent playback subgraphs |
| Offline render | Correctness/throughput | Larger blocks, more parallelism, faster than realtime |
| Plugin realtime-required render | Compatibility | Throttle to realtime and supply correct transport clock |

Increasing buffering can make serial work schedulable but increases interaction latency. Splitting a serial plugin chain across blocks can improve throughput only by adding pipeline latency. Such changes are modes or user-visible policy, never hidden “optimizations.”

## Explicit non-goals

- Maximizing OS CPU percentage.
- Assuming plugins, drivers, or disks behave ideally.
- Making UI state the source of engine timing.
- Synchronously reflecting every edit inside the current quantum.
- General-purpose dynamic allocation or arbitrary callbacks in RT.
- Perfect parallelism for inherently serial signal paths.

## Performance definition

For deadline `D = frames / sample_rate`, success is `Tgraph + Tio_overhead < D` with operating headroom across long runs. Track p50, p95, p99.9, maximum, and underruns. Averages alone cannot establish RT safety.

Primary targets:

- 48,000 / 64: 1.333 ms nominal callback period.
- 48,000 / 128: 2.667 ms.
- Sustained deadline utilization SHOULD remain below a platform-calibrated budget (initially 70% p99.9) to absorb OS/driver/plugin jitter; the test suite determines the supported promise.

## Priority order

1. Audio correctness
2. No real-time deadline violations
3. Deterministic behavior
4. Low latency
5. Multicore scalability
6. Memory safety and predictable allocation behavior
7. CPU efficiency
8. Maintainability
9. UI responsiveness
10. Implementation convenience

## References

- Ableton, [Multi-core performance in Ableton Live FAQ](https://help.ableton.com/hc/en-us/articles/209067649-Multi-core-performance-in-Ableton-Live-FAQ) — signal-path serialization, critical path, deadline-based load.
- PipeWire, [Graph Scheduling](https://docs.pipewire.org/page_scheduling.html) — preconfigured graph resources and separate control/data processing threads.

