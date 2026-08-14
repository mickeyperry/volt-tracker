# Engine Architecture

## Component ownership

| Subsystem | Owns | Executes on | Must not own |
|---|---|---|---|
| Audio backend | device stream, format conversion, clock observations | callback + backend control thread | project model, file access |
| RT coordinator | quantum context, graph generation pin, deadline, worker release/join | hardware callback | graph compilation, UI |
| DSP scheduler/workers | runnable nodes, dependency counters, node execution | fixed RT worker pool | arbitrary background jobs |
| Graph compiler | validation, node plan, PDC, buffers, schedule metadata | control/background | live mutable RT graph |
| Timeline engine | canonical sample clock, tempo-map projection, transport transition plan | control prepares; RT consumes | playhead pixels |
| Event compiler/router | timestamp conversion, sorted bounded event slices | control/input producer + RT consume | UI-frame timing |
| Plugin host | discovery database, instances, lifecycle, buses, state, GUI bridge | management/control; process on DSP workers | trust in plugin correctness |
| Stream manager | read-ahead, decode, seek generations, recording queues | disk/decode workers | synchronous callback reads |
| Render/cache | offline graph instances, freeze/bounce artifacts, validity keys | render workers | mutation of live instances |
| Project persistence | versioned document, migrations, atomic save | save/autosave workers | RT snapshots as serialization model |
| UI adapter | commands, immutable view snapshots, telemetry consumption | main/UI | timing-critical authority |

## State layers

1. **Project document**: mutable control-domain representation; stable IDs; serializable.
2. **Build input snapshot**: immutable copy/version of relevant project intent.
3. **Prepared graph**: executable nodes, edges, buffers, event routing, PDC, schedule ranks.
4. **Runtime state**: delay lines, oscillator/plugin state, stream cursors; RT-owned or node-owned.
5. **Telemetry snapshot**: lossy observation; never fed back as timing truth.

Do not conflate project identity with runtime addresses. Prepared graphs reference stable resource handles whose lifetimes extend past graph retirement.

## Edit-to-audio transaction

```
UI command
  -> control model validates edit
  -> capture project revision R
  -> graph compiler builds G(R), reserves all RT resources
  -> reject or publish G with generation N
  -> coordinator pins N at next quantum boundary
  -> old graph retires after no callback/worker can reference it
  -> non-RT reclamation destroys graph and plugin resources
```

Coalesce obsolete graph builds. Publication is atomic; activation may require a prepared transition plan. Never destruct a retired graph on RT. Crossfade topology changes only when necessary and with preallocated state; otherwise swap at a sample/quantum boundary with defined discontinuity handling.

## Execution flow per quantum

1. Backend supplies input/output spans and timing metadata.
2. Coordinator pins graph, transport, and parameter snapshot generations.
3. Reset preallocated dependency counters and seed ready nodes.
4. Coordinator and workers execute runnable nodes until required sinks finish.
5. Convert/copy graph sinks into backend buffers.
6. Publish compact metrics, advance canonical sample state, release generation pin.

Only nodes contributing to active sinks/tails need execute. Silence propagation MAY skip nodes only if their contract says silence-in yields silence-out and no tail/state/event work is required.

## Failure boundaries

| Failure | Required behavior |
|---|---|
| Graph compile invalid/capacity exceeded | Keep previous graph; report structured error |
| Worker misses deadline | Complete/copy best bounded fallback, count underrun; never wait forever |
| Plugin faults/hangs | In-process faults may terminate audio/process; optional sandbox enables bypass/restart |
| Stream starvation | Supply silence/fade policy, flag stream and metric, preserve timeline |
| Device change/loss | Stop backend safely, negotiate off-RT, rebuild format-dependent graph, restart |
| Telemetry queue full | Drop/coalesce telemetry; audio unaffected |
| Command queue full | Reject/coalesce noncritical command visibly; no unbounded growth |

## Compatibility with current VOLT

The current browser project and Rust transliteration are implementation stages, not exceptions to this architecture. Browser Web Audio may hide its internal graph/callback scheduling; native engine work still follows these contracts. Existing empirically verified plugin-host lifecycle constraints remain authoritative until replaced by stronger tests.

## References

- JUCE, [AudioProcessorGraph](https://docs.juce.com/master/classAudioProcessorGraph.html).
- PipeWire, [Graph Scheduling](https://docs.pipewire.org/page_scheduling.html).
- Ardour source, [ardour/ardour](https://github.com/Ardour/ardour).

