# Memory Model

## Allocation policy

| Phase/domain | May allocate? | Rules |
|---|---|---|
| Startup/control/project edit | Yes | bounded validation; failures leave live graph intact |
| Graph compile/prepare | Yes | reserve all graph, buffer, event, scratch, PDC and transition capacity |
| Plugin create/activate | Yes, off-RT | assume arbitrary allocation/blocking |
| Disk/decode/waveform/save | Yes | bounded queues/backpressure; never share allocator locks with RT where avoidable |
| Hardware callback/DSP workers | **No** during normal processing | pools/spans/fixed capacity only; deferred destruction |
| Offline render | Yes outside live RT | prefer reuse; cannot interfere with live deadline |

“No allocation” includes hidden growth, boxed error creation, string formatting, first-use lazy tables, TLS initialization, shared-pointer final release, and plugin-host adapter containers.

## Pools and layout

- **Audio pool**: aligned planar floating-point blocks sized for maximum prepared quantum/channels. Graph liveness assigns handles and reuses storage after last consumer.
- **Scratch pool**: per-worker fixed arenas reset by index each quantum; node declares maximum scratch during prepare.
- **Plugin buffers**: host-owned port pointer arrays and writable buffers prepared for maximum buses/channels/block. Do not expose shared writable storage across plugins.
- **Event pool**: fixed-capacity contiguous headers plus payload storage; sorted by sample offset and stable sequence.
- **Delay/PDC pool**: persistent node-owned rings sized from maximum declared latency plus block margin.

Align SIMD audio starts appropriately (at least implementation SIMD requirement); align independently written worker metadata to cache lines. Prefer contiguous arrays/indices over fragmented heap nodes. Measure padding: alignment that prevents false sharing can waste cache when applied blindly.

## Ownership

| Object | Owner | RT access | Reclamation |
|---|---|---|---|
| Project document | control | none | control |
| Prepared graph | graph generation manager | read-only + runtime handles | non-RT after all pins clear |
| Node DSP state | one active graph/node | exclusive during node process | non-RT after retirement |
| Audio buffer block | graph pool | compiler-defined producer/consumers | graph retirement/reuse by liveness |
| Stream ring | stream object | producer disk/decode; consumer RT (or inverse for record) | stopped/drained off-RT |
| UI snapshot | UI adapter | RT writes bounded alternate slot only if designed | UI/control |

Never return an RT-visible raw pointer whose backing store can move. Stable handles resolve through immutable tables pinned for the quantum.

## Ring buffers and queues

Use SPSC rings for one stream producer/consumer; they are simpler and cheaper than general MPMC. Head/tail are monotonic counters on separate cache lines. Producer publishes payload then head (release); consumer reads head (acquire), consumes, then publishes tail. Define wrap and counter overflow using unsigned arithmetic. Queue capacity and full policy are part of correctness.

For many disk streams, per-stream rings give isolation but memory/seek overhead; grouped/interleaved cache pages reduce I/O and memory. Choose from measured stream count and edit locality.

## Snapshots

Snapshots are immutable after publication. Publication carries a generation and release/acquire ordering. Large parameter arrays should use chunked copy-on-write or compiled event slices so one knob change does not copy megabytes. Do not read a seqlock-protected large object on RT if writers can force unbounded retries.

## Buffer clearing and reuse

Clearing is real CPU/memory bandwidth. Mark whether nodes fully overwrite outputs; clear only required regions. Silence propagation can bind read-only zero pages/spans for trusted native nodes. For third-party plugins, supply valid writable buffers according to format requirements and guard canaries in diagnostic builds.

## Exhaustion behavior

- Graph preparation capacity failure: reject build with required/available counts.
- RT event overflow: deterministic priority/drop rule and telemetry; note-off and safety events outrank cosmetic control data.
- Playback ring empty: bounded silence/de-click policy and starvation metric.
- Recording ring full: flag take corruption immediately; never overwrite unread capture silently.
- Telemetry full: overwrite/coalesce/drop.

## Verification

Instrument allocation counts by thread; run callback under allocator traps in tests. Use sanitizers outside strict RT benchmarks, pool canaries, randomized graph lifetimes, and generation retirement stress. Report pool high-water marks.

## References

- PortAudio, [Writing a Callback Function](https://portaudio.com/docs/v19-doxydocs/writing_a_callback.html).
- CLAP, [ABI and extensions](https://github.com/free-audio/clap).

