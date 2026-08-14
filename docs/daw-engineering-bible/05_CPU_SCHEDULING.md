# CPU Scheduling and Deadline Execution

## Objective

Minimize worst-case required-sink completion time, especially the critical path. Total work matters when all cores saturate; critical-path length and scheduling overhead matter even when OS CPU appears low.

```
 A(0.3ms) --+                   +-> Master(0.2)
            +-> Bus(0.4) -------+
 B(0.8ms) --+                   |
 C(0.2ms) -> FX1(0.7) -> FX2(0.7)+
                         ^ critical serial branch
```

If the 1.6 ms branch is required for a 1.33 ms quantum, the project underruns while other cores may be idle. CPU percentage averages capacity across cores/time; audio needs the current graph complete now.

## Prepared scheduling metadata

For node `i`, store contiguous: initial dependency count, successor range, downstream critical rank, estimated cost, affinity hint, buffer handles, callable/process index. Cost estimates are rolling robust statistics capped/decayed and never trusted for correctness.

## Quantum algorithm

1. Reset dependency counters from a preallocated template (or generation-stamped counters).
2. Enqueue active zero-dependency nodes.
3. Workers pop highest-useful-priority runnable nodes and process exactly once.
4. On completion, publish outputs; decrement each active successor dependency.
5. The thread observing zero enqueues that successor.
6. Finish when required sink completion count reaches zero, not after unrelated optional nodes.

Priority SHOULD consider downstream critical rank, estimated node cost, sink urgency, cache affinity, and starvation fairness. Validate that priority bookkeeping costs less than it saves.

## Fan-in, fan-out, and barriers

Fan-out exposes parallelism after one producer. Fan-in is a necessary wait point. A global phase barrier manufactures unnecessary waits:

```
Bad: all tracks | barrier | all buses | barrier | master
Good: each bus releases when its own predecessors finish
```

Large successor fan-out can cause atomic/cache bursts. Use compact successor arrays and measured batching without delaying critical successors.

## Serial bottlenecks

Long plugin chains, oversampled instruments, master limiters, nested buses, sidechains, and returns can dominate. Valid remedies: optimize/replace the node, freeze/bounce, share a return where semantically correct, increase device block size, use playback lookahead, or explicitly pipeline with added latency. Invalid remedy: process dependent effects simultaneously on the same samples.

## Deadline behavior

Compute `deadline = frames / sample_rate`, but measure effective budget from callback timestamps and backend safety margin. Track elapsed time at low overhead. Do not launch optional/lookahead work if the remaining budget cannot accommodate it. Background pools yield resources while low-buffer playback is active.

Scheduler overhead is part of the deadline: queue operations, atomics, wakeup, migrations, buffer clearing, denormal handling, and final conversion. Small nodes may be statically fused into a scheduling cluster to amortize overhead, trading parallel granularity for locality. Cluster only when profiling shows benefit and do not hide PDC/event boundaries.

## Anticipative scheduling

Maintain a separate non-RT/lookahead executor for eligible playback subgraphs. It renders future sample ranges into bounded caches. Eligibility is invalidated by live input/MIDI, active recording, externally mutable plugin state, transport uncertainty, or edits touching the range. RT consumes a cache block only when its complete validity key matches; otherwise it computes normally. Do not let speculative workers compete with current-deadline workers.

## Measurement requirements

- Graph completion and callback durations; deadline utilization distribution/max.
- Per-node inclusive process time and variance, sampled to limit observer cost.
- Critical path reconstructed from actual dependencies and measured costs.
- Worker useful/busy/spin/sleep time; wake and steal latency.
- Ready-queue depth, atomics/steals, scheduling overhead, migrations if observable.
- Compare single-worker correctness and multiworker output determinism.

## References

- Ableton, [Multi-core performance FAQ](https://help.ableton.com/hc/en-us/articles/209067649-Multi-core-performance-in-Ableton-Live-FAQ).
- PipeWire, [Graph Scheduling](https://docs.pipewire.org/page_scheduling.html).

