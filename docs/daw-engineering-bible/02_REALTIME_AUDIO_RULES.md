# Real-Time Audio Rules

## Hard invariant

The RT domain must finish every required output quantum before the device deadline. “Usually fast” is insufficient: every operation must have a known bounded path or a measured, explicitly accepted third-party risk.

## Forbidden in the RT domain

- Filesystem, network, database, IPC request/response, plugin scanning, file loading, synchronous decoding.
- General heap allocate/free, reference-count destruction that can cascade, allocator-backed container growth.
- Mutexes, condition-variable waits, blocking semaphores, thread creation/join, sleeping, OS calls with unbounded latency.
- UI calls, event-loop dispatch, clipboard/window work, project serialization, autosave.
- Graph construction, routing validation, PDC recomputation, large state copy, plugin creation/destruction/activation.
- Formatting strings, stack traces, console/file logging, exceptions/unwinding across the callback.
- Page faults caused by untouched memory where memory locking/prefaulting is available and justified.
- Unbounded loops, retries, queue draining, recursion based on project/plugin input.

Third-party `process` calls necessarily enter RT and may violate these rules internally. Treat that as a measured compatibility risk; isolate where product policy requires stronger containment.

## Required RT shapes

- Fixed-capacity or pre-reserved buffers, queues, and event lists.
- Spans/views over owned storage; no ownership transfer that can destroy on RT.
- Bounded MPSC/SPSC communication with explicit full/empty policy.
- Immutable snapshots published atomically and reclaimed outside RT after a grace period.
- Precomputed routing, latency, buffer aliases, node ranks, and worst-case capacities.
- Cache-line-separated hot atomics/counters; contiguous audio/event data.
- Denormal control or explicit tiny-value handling in DSP where supported.
- Compact numeric telemetry records; format them later.

## Communication choices

| Need | Preferred mechanism | Full/contention behavior |
|---|---|---|
| UI/control commands | bounded SPSC/MPSC queue | coalesce replaceable values; reject structural edit with visible status |
| Parameter latest value | atomic scalar plus generation, or bounded event queue for sample accuracy | latest-value is not valid for recorded automation |
| Graph/state publication | atomic pointer/index to immutable generation | retain old generation until safe |
| Meters/telemetry | SPSC ring or double-buffer snapshot | overwrite/drop old observations |
| Disk playback/recording | SPSC audio ring per stream/group | silence/fade on playback starvation; mark recording overrun |

“Lock-free” does not mean wait-free, fair, cache-efficient, or correct. CAS retry storms, reclamation, false sharing, and unbounded producer loops can be worse than a mutex. Use ordinary locks in the control/background domain when they simplify ownership. Use lock-free structures at an RT boundary only with a proof of bounded RT operations and a stress test.

## Snapshot reclamation

Acceptable strategies include epoch/grace-period reclamation, fixed graph slots, or control-owned `Arc`/reference management where RT increments/decrements are proven bounded and final destruction is deferred. An atomic pointer swap alone is unsafe: the publisher must not free a snapshot still pinned by callback/workers.

## Overload policy

The scheduler must not extend work indefinitely to “catch up.” On overload:

1. Count and timestamp the missed/near-missed deadline with preallocated telemetry.
2. Preserve clock/timeline progression.
3. Use a predefined backend-safe output policy (completed buffer, silence, or short de-clicked fallback).
4. Never silently drop recording input; mark the take invalid/partial and surface it.
5. Degrade only explicitly optional work already outside the audible graph (meters, analysis, waveform, cache).

## RT code review checklist

- Is call depth and iteration count bounded by prepared capacities?
- Can any destructor, callback, plugin, system API, or lazy initialization allocate/block?
- Are buffers touched/prefaulted and sufficiently aligned?
- What happens when every queue is full and every pool is exhausted?
- Is object lifetime valid through the last worker completion?
- Are memory orderings documented at publication/queue boundaries?
- Does instrumentation itself remain bounded and allocation-free?
- Has the code been tested with an allocation/block detector in the callback?

## References

- JACK, [Designing & Writing a JACK Client](https://jackaudio.org/api/index.html).
- PortAudio, [Writing a Callback Function](https://portaudio.com/docs/v19-doxydocs/writing_a_callback.html).
- Apple, [Audio Unit Hosting Guide](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/AudioUnitHostingGuide_iOS/Introduction/Introduction.html).
- PipeWire, [Graph Scheduling](https://docs.pipewire.org/page_scheduling.html).

