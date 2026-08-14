# Engine Decision Rules

## Permanent rule

> **Never solve a local performance problem in isolation. Consider its effect on the complete real-time audio graph, dependency critical path, cache behavior, memory allocation, synchronization, thread scheduling, latency and multicore scalability.**

This checklist is a merge gate for every substantial engine change. Put answers in the design note, issue, or change description. “No effect” requires a reason or measurement.

## Mandatory questions

1. **Which subsystem owns this responsibility?** Name one primary owner and any interfaces; reject ambiguous shared ownership.
2. **Which thread or threads execute it?** Include creation, steady state, error, teardown, and callbacks.
3. **Does it run in the real-time domain?** Identify hardware callback, DSP worker, or neither.
4. **Can it allocate?** Include hidden growth, lazy initialization, formatting, destructors, plugin/system calls. State preparation capacity.
5. **Can it block?** Include locks, I/O, IPC, page faults, waits, retries, process/window calls.
6. **Does it require synchronization?** Name producer/consumer, primitive, memory ordering, boundedness, contention, and full/empty behavior.
7. **How does it affect the DSP dependency graph?** List nodes/edges, fan-in/out, sidechains, sends, event or feedback dependencies.
8. **Does it lengthen the critical path?** Estimate and then measure current and proposed path cost.
9. **Does it increase latency?** Account for algorithmic, buffering, PDC, monitoring, capture, render, and UI-observation latency.
10. **What is its worst-case execution behavior?** Bound iterations, data size, third-party calls, jitter, and first-use/cold behavior.
11. **What happens under overload?** Specify queue/pool exhaustion, missed deadline, plugin spike, disk starvation, cancellation, and user-visible result.
12. **How is correctness tested?** Include sample boundaries, routing/PDC, lifecycle, concurrency, failure injection, and determinism/tolerance.
13. **How is performance benchmarked?** Name `14_PERFORMANCE_TESTS.md` matrix cells, metrics, baseline, duration, hardware, and regression threshold.
14. **Can the work be moved outside the real-time path?** If not, explain causal deadline need.
15. **Can the work be cached, precomputed or processed ahead?** State validity key, invalidation, memory budget, and live-input restrictions.

## Decision priority

Resolve conflicts in this order:

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

A lower item cannot override a higher item without an explicit product-level change to this contract. “Cleaner code” does not justify callback allocation; “more parallel” does not justify extra monitoring latency without a mode decision.

## Required change record template

```text
Change:
Owner/subsystems:
Execution domains and lifecycle:
RT entry points:
Allocation/blocking/synchronization proof:
Graph edges and critical-path impact:
Latency/PDC impact:
Worst case and overload policy:
Prepared/cached/lookahead alternatives:
Correctness tests:
Performance matrix, baseline, result:
Telemetry/diagnostics:
Compatibility/migration/rollback:
Uncertainties and primary sources:
```

## Immediate rejection conditions

- Filesystem/decoding/scanning/UI/save/graph-build work added to callback or DSP workers.
- General allocation/free or blocking synchronization added to normal RT processing.
- Mutable graph/state publication without a complete lifetime/reclamation scheme.
- New routing behavior without dependencies, cycle handling, PDC, and event semantics.
- “One track = one thread” scheduling or a global barrier where dependency release suffices.
- Performance claim based only on average CPU or a short successful playback.
- Unbounded queue/pool, retry loop, logging, or failure recovery.
- Plugin lifecycle/control raced with processing or unknown plugin loaded during scan in main process.
- UI timestamps used as musical timing authority.
- Cache/freeze accepted without dependency-complete validity and invalidation.
- Optimization benchmarked without correctness equivalence and primary target cells.

## Alternatives must be compared honestly

| Choice | Required comparison |
|---|---|
| Lock-free vs lock | bounded RT operations, contention, reclamation, cache traffic; mutex is often better outside RT |
| Fine nodes vs fused clusters | parallelism vs queue/atomic overhead and locality |
| More DSP workers | useful physical cores vs oversubscription, SMT, wake/barrier cost |
| In-process vs sandbox plugin | latency/transport cost vs crash/hang containment |
| Small vs large device block | interaction latency vs deadline margin/callback overhead |
| Stream vs preload | I/O starvation/seek behavior vs memory footprint |
| Live compute vs lookahead/cache | edit/live-input validity and memory vs reduced peak RT work |
| In-place vs separate buffers | bandwidth/pool savings vs fan-out safety and plugin guarantees |

## Completion criteria

A substantial engine change is complete only when:

- ownership and execution domain match `01` and `04`;
- RT review passes `02` and memory high-water/allocation evidence matches `06`;
- graph, timing, plugin, stream, and UI contracts affected by it are tested;
- primary performance cells show no unexplained regression;
- telemetry can diagnose failure in a user project;
- documentation is updated if the architecture—not merely implementation—changed.

