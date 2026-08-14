# Dependency-Aware DSP Graph

## Graph model

Compile project routing into a directed acyclic graph (DAG) per processing mode. Nodes expose typed audio/event inputs and outputs, declared latency/tail, in-place capability, silence behavior, and a bounded `process(context)` operation. Edges express data dependencies, not UI containment.

```
Audio clip -> Track FX ----+----> Group FX ----+
                           |                    |
Instrument -> Track FX ----+ (sum)              +--> Master FX -> output
        \---- sidechain ----------------------> compressor
             send ----> Return FX -------------+
```

## Dependency rules

- A serial plugin chain is one edge sequence; nodes cannot reorder across stateful processing.
- A bus/return/summing node depends on all audible upstream contributors for that quantum.
- A sidechain processor depends on main and sidechain-producing paths.
- A send creates a dependency from its tap point to the return; pre/post-fader selects the source node.
- Instruments depend on their input event slice and transport context; audio effects depend on audio plus parameter events.
- Master processing depends on every routed master contributor and required return.
- Automation and MIDI are timestamped data dependencies. Preparing events off-RT does not force DSP serialization unless routing does.
- PDC delay nodes/state are explicit. A shorter path is delayed to align at a convergence; compensation latency changes graph preparation and buffer requirements.
- Muted/bypassed nodes may be elided only under documented semantic rules for tails, latency, automation, and plugin state.

## Feedback

An instantaneous cycle has no topological schedule and MUST be rejected. Supported feedback requires an explicit delay of at least one defined quantum/sample policy, converting the cycle into cross-quantum state:

```
A(n) -> B(n) -> DelayState.write(n)
          ^       |
          +-- DelayState.read(n-1)
```

The UI must state the inserted delay. Never “break” cycles using accidental scheduling order.

## Compilation pipeline

1. Capture immutable project revision and runtime format.
2. Normalize routing into typed nodes/ports; resolve stable resource IDs.
3. Validate channels, bus layouts, missing endpoints, and prohibited zero-delay cycles.
4. Add mixers, format converters, event routers, explicit feedback delays, and PDC delays.
5. Query/freeze node latency and tail declarations for this generation.
6. Compute topological order, predecessor/successor arrays, downstream critical ranks.
7. Perform liveness analysis and allocate/alias buffers without violating in-place rules.
8. Reserve event/scratch capacities and initialize nodes outside RT.
9. Produce immutable arrays and a transition/retirement plan.
10. Publish only after all validation and preparation succeeds.

Prefer structure-of-arrays scheduling metadata: contiguous node descriptors, successor ranges, dependency counters, ranks, and buffer handles. Avoid pointer-chasing object graphs in hot scheduler paths.

## Latency compensation

For each node, compute arrival latency as the maximum predecessor arrival plus predecessor/node latency according to a single documented convention. At fan-in, insert delays on earlier arrivals to equal the maximum. Recompute when routing, oversampling, plugin latency, sample rate, or relevant mode changes.

PDC correctness includes sidechains and sends. Aligning only main audio can alter compressor response or wet/dry phase. Live-monitored paths may use a low-latency policy that bypasses high-latency nodes or limits compensation; this is explicit and separately tested.

## Buffer semantics

- Node ports use non-owning planar spans for the quantum.
- Fan-out output remains immutable until all consumers finish unless compiler assigns separate copies.
- In-place aliasing requires one consumer, compatible format, and a plugin/node guarantee.
- Sum nodes write a distinct cleared buffer or safely reuse one dead input after liveness proof.
- Silence buffers are read-only; never give an untrusted plugin a shared global silence buffer as writable output.

## Dynamic changes

Parameter/event changes do not rebuild topology. Structural edits do. Compile asynchronously and swap generations at a quantum boundary. Preserve state only via explicit compatible handoff; never concurrently process one stateful instance in old and new graphs. Crossfade using separate prepared instances when discontinuity risk warrants its CPU/memory cost.

## Anticipative processing

Playback-only subgraphs may run ahead if all inputs are known and no descendant depends on live input, uncommitted control, nondeterministic external transport, or a plugin requiring current wall-clock behavior. Cache results tagged by graph/project/tempo/automation/sample-range generations. Maintain a ready horizon, but bound memory and cancel stale work. At the live boundary, consuming prepared blocks reduces peak RT work; missing lookahead falls back to normal RT processing.

Lookahead adds state-management complexity and possibly latency. It does not make causally live paths parallel and must never render past an unresolved edit as though it were current.

## References

- JUCE, [AudioProcessorGraph](https://docs.juce.com/master/classAudioProcessorGraph.html).
- PipeWire, [Graph Scheduling](https://docs.pipewire.org/page_scheduling.html).
- Ableton, [Multi-core performance FAQ](https://help.ableton.com/hc/en-us/articles/209067649-Multi-core-performance-in-Ableton-Live-FAQ).

