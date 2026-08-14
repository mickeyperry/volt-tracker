# DAW Engineering Bible

Permanent architecture contract for VOLT's professional native audio engine. These documents describe the target design; they do not imply that every component is implemented. Requirements use **MUST**, recommendations **SHOULD**, and allowed choices **MAY**.

## How coding agents must use this documentation

Before modifying engine code, an agent **must read the documents relevant to the change**, identify the owning subsystem and execution domain, and complete the checklist in [15_ENGINE_DECISION_RULES.md](15_ENGINE_DECISION_RULES.md). An agent must not violate documented real-time invariants merely to make a local implementation easier.

For a substantial change, record: affected graph edges, RT allocation/blocking behavior, latency change, overload behavior, correctness tests, and benchmark. If implementation and this pack disagree, stop and resolve the architectural decision explicitly; do not silently weaken the contract.

## Reading map

| Task | Required reading |
|---|---|
| Any engine architecture change | `00`, `01`, `02`, `15` |
| DSP/effect/instrument/routing | `02`, `03`, `04`, `05`, `06`, `10`, `11` |
| Threads, workers, performance | `02`, `04`, `05`, `06`, `14`, `15` |
| Plugin host, scan, GUI, state | `02`, `03`, `06`, `07`, `13` |
| Audio device/backend/latency | `02`, `04`, `08`, `14` |
| Clip playback, recording, storage | `02`, `06`, `08`, `09`, `10` |
| Transport, tempo, automation | `03`, `10`, `11` |
| MIDI or parameter events | `03`, `07`, `10`, `11` |
| Freeze, bounce, render cache | `03`, `07`, `10`, `12` |
| UI meters, playhead, commands | `02`, `10`, `13` |
| Tests/profiling/optimization | `05`, `14`, `15` |

## Canonical terminology

- **RT domain**: hardware callback/coordinator plus DSP workers participating in the current device deadline.
- **control domain**: non-RT threads that mutate project intent and compile engine state.
- **processing quantum**: frames processed as one scheduler invocation; normally the device callback block.
- **prepared graph**: immutable, validated, latency-resolved executable graph built outside RT.
- **graph snapshot**: one prepared graph generation visible to RT for a whole quantum.
- **node**: schedulable DSP unit with declared audio/event ports, latency, tail, and state.
- **critical path**: longest time-weighted dependency path required to produce the current output quantum.
- **deadline utilization**: graph completion time / callback deadline; this, maxima, and tails matter more than aggregate CPU.
- **PDC**: plugin/processing delay compensation implemented as explicit delay state/edges.
- **command**: bounded control-to-engine message; **telemetry** is bounded engine-to-control/UI observation.

## System-wide invariants

1. Audio correctness and deadline safety outrank feature convenience.
2. The callback consumes prepared audio, events, buffers, and graph state; it does not discover or construct them.
3. Independent nodes may execute concurrently. A node executes only after every dependency for that quantum is complete.
4. RT code does not perform filesystem access, heap allocation/free, blocking waits, mutex acquisition, graph construction, plugin scanning, expensive logging, or UI work.
5. State crosses domains through bounded queues, atomics, or immutable snapshots with explicit lifetime reclamation.
6. Every failure path is bounded. Overload produces a defined degradation or dropout, never unbounded waiting or memory growth.
7. All timing is internally sample-accurate. UI pixels and frame cadence are projections, never timing authority.

## Document index

| File | Contract |
|---|---|
| `00_PRODUCT_VISION.md` | Product constraints and success measures |
| `01_ENGINE_ARCHITECTURE.md` | Components, ownership, data flow |
| `02_REALTIME_AUDIO_RULES.md` | Hard RT invariants and review rules |
| `03_DSP_GRAPH.md` | Graph semantics, compilation, PDC |
| `04_THREADING_MODEL.md` | Threads and execution domains |
| `05_CPU_SCHEDULING.md` | Deadline-aware DAG scheduler |
| `06_MEMORY_MODEL.md` | Pools, ownership, allocation policy |
| `07_PLUGIN_ARCHITECTURE.md` | VST3/CLAP/AU host lifecycle and isolation |
| `08_AUDIO_IO.md` | Devices, callbacks, clocks, formats |
| `09_DISK_STREAMING.md` | Playback and recording streams |
| `10_TIMELINE_TRANSPORT.md` | Sample clock, tempo, loop, record |
| `11_MIDI_EVENT_SYSTEM.md` | Timestamped event model and routing |
| `12_RENDERING_FREEZE_CACHE.md` | Offline render and invalidation |
| `13_UI_ENGINE_SEPARATION.md` | Commands, snapshots, telemetry |
| `14_PERFORMANCE_TESTS.md` | Reproducible benchmark/stress suite |
| `15_ENGINE_DECISION_RULES.md` | Mandatory change checklist |

