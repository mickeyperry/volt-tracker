# Performance and Stress Tests

## Benchmark contract

Tests are reproducible artifacts: record build/commit, OS, power mode, CPU/topology, RAM, storage, audio interface/driver/API/version, actual sample rate/block, plugin binaries/presets, project seed, warmup, duration, and background conditions. Disable debug/sanitizer overhead for release performance runs; run correctness/safety builds separately.

Primary acceptance target is 48 kHz at 64 and 128 frames on declared reference hardware. Never publish “track count” without project topology, plugin workload, and deadline-tail statistics.

## Matrix

Run supported combinations; bold cells are release gates:

| Sample rate | Buffer frames |
|---|---|
| 44.1 kHz | 32, 64, 128, 256, 512, 1024 |
| 48 kHz | **32, 64, 128**, 256, 512, 1024 |
| 96 kHz | 32, 64, **128**, 256, 512, 1024 |

Unsupported device combinations are recorded as unsupported, not failed engine tests. Use a deterministic simulated callback harness for the complete matrix and real devices for their supported cells.

## Required metrics

- callback and graph duration: p50/p95/p99/p99.9/max;
- callback deadline and utilization distribution/max;
- internal deadline misses and backend underrun/xrun counts;
- per-node/plugin execution distribution and reconstructed critical path;
- worker useful/busy/spin/sleep, wake latency, steals/migrations where available;
- scheduling overhead and ready-queue depth;
- disk ring minimum fill/starvation, read/decode latency and cache hit rate;
- buffer/event/scratch pool high-water and exhaustion;
- graph compile/publish/retire time and memory;
- RT and total allocation counts; CPU/package usage as secondary context.

Instrumentation overhead must be benchmarked on/off. No expensive tracing in every callback in release mode.

## Workloads

| ID | Scenario | Construction and assertion |
|---|---|---|
| P01 | Many independent audio tracks | 1..N streamed clips, identical native FX, direct master; expect scaling until cores/bandwidth saturate |
| P02 | Extremely long serial chain | 1 track, 1..N costly stateful FX; expose critical-path limit and low aggregate CPU underrun |
| P03 | Many software instruments | seeded dense polyphony across independent instruments; verify event/sample output and voice spikes |
| P04 | Nested buses | balanced and skewed 1..5 levels; no global phase barriers; PDC correct |
| P05 | Multiple sends | pre/post sends to shared returns; validate wet alignment, fan-in scheduling |
| P06 | Sidechains | many compressors with independent/ shared keys; verify dependency and PDC |
| P07 | Hundreds of plugin instances | representative VST3/CLAP/AU mix; scan excluded; process/lifecycle stability |
| P08 | Automation-heavy | dense ramps/steps at and within block boundaries; bounded event capacity/no allocations |
| P09 | MIDI-heavy | dense chords, CC, bends, plugin event routing; stable order/no stuck notes |
| P10 | Disk streaming | many long PCM/compressed streams, cold/warm cache, injected I/O jitter and seeks |
| P11 | Simultaneous recording | maximum supported inputs plus playback; writer stalls/disk-full injection; exact placement |
| P12 | UI under DSP load | continuous scrolling/zoom/meters/waveform; block UI; callback unchanged |
| P13 | Plugin GUI stress | open/resize/hide many editors during playback; no RT waits or lifecycle races |
| P14 | Seek/loop stress | rapid random seeks and short loops at tempo/automation boundaries; no stale generations |
| P15 | Graph modifications | add/remove/reroute/bypass repeatedly during playback; atomic generations, no UAF/click beyond policy |
| P16 | Low-buffer operation | 32/64 frames for >=30 min with mixed realistic graph; tail latency and thermal stability |

## Test shapes and controls

- Run independent-width sweeps and serial-depth sweeps separately.
- Include balanced and one-heavy-node distributions to test stealing/priority.
- Native deterministic DSP establishes scheduler baseline; curated plugins measure real-host risk.
- Run single-worker reference and multiworker versions; compare rendered output hashes/tolerances.
- Warm up plugin/JIT/lazy paths, then also test cold start/first playback explicitly.
- Minimum duration: short capacity sweeps >=60 s/cell; release soak >=30 min primary cells; overnight scenario rotation before major releases.
- Repeat runs (>=5 capacity, >=3 soak) and report median plus worst; do not discard outliers without a recorded external cause.

## Fault injection

Inject worker delay, plugin spike/hang in sandbox, queue/pool exhaustion, stream latency/partial read, device discontinuity, clock drift, allocation trap, graph compile failure, stale cache, and UI pause. Assert defined degradation from each subsystem document.

## Gates

Initial gates, calibrated per reference hardware and product promise:

- zero underruns in primary target soak tests;
- p99.9 deadline utilization <=70% and max <100% absent documented external interruption;
- zero host allocations/frees on RT after warmup (plugin internals reported separately);
- no unbounded queue growth; no pool exhaustion in supported-capacity projects;
- sample-accurate correctness suites pass before performance result is accepted;
- change regression thresholds: <=3% p99.9 callback/critical-path/scheduler overhead unless justified by recorded architecture decision.

Capacity failure must be graceful and measurable. A test that merely “plays without crashing” is insufficient.

## Result record

Store machine-readable JSON/CSV plus a concise report with topology diagram, maximum supported load by gate, limiting critical path, regressions, and raw artifact paths. Compare distributions, not only averages.

