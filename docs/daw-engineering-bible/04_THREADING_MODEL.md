# Threading Model

## Recommended threads

| Thread/pool | Count | Priority/affinity | Responsibilities | Forbidden |
|---|---:|---|---|---|
| Hardware callback / RT coordinator | backend-defined | highest supported audio class; prefer P-core | pin snapshots, seed/participate in graph, deliver I/O | blocking, allocation, control work |
| DSP workers | calibrated fixed pool | RT/audio class below/equal coordinator; P-cores | execute ready graph nodes | non-DSP jobs, sleep during active quantum unless bounded primitive is proven |
| UI/main | 1 | normal | windows, input, drawing, plugin GUI coordination | engine timing authority |
| Control/graph compiler | 1 or small pool | normal/high, non-RT | project commands, graph builds, state publication | DSP deadline participation |
| Disk streaming | small dedicated pool | elevated I/O, non-RT | read-ahead, writes, seek requests | decode-heavy CPU monopolization |
| Decode/background | bounded pool | normal/low; E-cores acceptable | codec decode, waveform, analysis, cache | RT work |
| Plugin management/scanning | isolated workers/processes | normal/low | discovery, instantiate probes, metadata | callback interaction |
| Save/autosave | 1 serial queue | low | snapshot serialization, temp-write + atomic replace | reading mutable RT state directly |
| Offline render | configurable separate pool | normal/high | independent render graph | sharing live stateful plugin instances |

Backend APIs may own an internal callback thread. Promote/tag only documented audio threads using platform facilities. Do not raise unrelated background work to RT priority.

## Pool sizing and topology

- Start with coordinator + one worker per available **physical performance core**, minus capacity needed by driver/system; calibrate rather than equating logical CPUs with cores.
- SMT siblings share execution/cache resources. Benchmark whether an additional DSP worker helps or increases tail latency.
- Prefer P-cores for coordinator and deadline workers on hybrid CPUs. E-cores are suitable for decode, analysis, save, and scan. When affinity APIs are unavailable, use platform QoS/priority hints and measure migration.
- Avoid oversubscription: more runnable RT threads than useful cores raises preemption, cache eviction, and barrier cost.
- Pool size is stable while streaming. Reconfigure only with audio stopped or through a prepared safe transition.

## Worker activation

Workers remain resident. At quantum start, coordinator updates a generation counter and publishes initial work with release semantics. Workers use bounded adaptive waiting: short calibrated spin/poll while a quantum is active, then a platform event/semaphore between callbacks. Pure spinning wastes cores and harms thermal/power behavior; sleeping on every tiny task can miss a 1.33 ms deadline.

No worker is allowed an unbounded OS wait while output depends on it. Coordinator may execute nodes itself and must have a bounded completion protocol. Measure wake latency on every supported backend/OS.

## Work stealing

Use per-worker deques plus a compact global/priority ready structure only if benchmarks beat a simpler shared bounded queue. Owner-local pop preserves cache locality; stealing balances uneven plugin costs. Prevent contention on one global head and prioritize nodes with high downstream critical rank. Stealing operations in RT must be bounded; no allocation and no indefinite CAS loops.

Affinity hints may keep stateful nodes on the prior worker for warm caches, but correctness cannot depend on thread identity unless a plugin contract requires it. A thread-affine plugin becomes a scheduling constraint and potential bottleneck.

## Cache and synchronization

- Align frequently written worker counters/queues to `hardware_destructive_interference_size` or a conservative measured cache-line size.
- Never place per-worker completion counters on one line (false sharing).
- Publish audio buffers through graph dependency release/acquire edges; do not add a global full fence after every node without need.
- Fan-in requires one atomic predecessor completion per incoming edge or an equivalent bounded scheme. One final decrement makes the successor runnable.
- Avoid a whole-graph barrier between “track” and “bus” phases; dependency counters naturally release buses as soon as their own inputs finish.

## Thread-safety assumptions

Each node instance is processed at most once concurrently. Plugin lifecycle/control methods follow format contracts and are never raced with `process`. State serialization is not called on an active instance unless the plugin API explicitly guarantees it; otherwise suspend or use a shadow/control representation.

## References

- Microsoft, [Multimedia Class Scheduler Service](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service).
- PipeWire, [RT module](https://docs.pipewire.org/page_module_rt.html).
- Ableton, [Multi-core performance FAQ](https://help.ableton.com/hc/en-us/articles/209067649-Multi-core-performance-in-Ableton-Live-FAQ).

