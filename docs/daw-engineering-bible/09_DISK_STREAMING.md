# Disk Streaming and Recording

## Contract

The audio callback consumes/produces memory already staged in bounded rings. It never opens, reads, writes, seeks, maps on demand, or decodes a file.

## Playback pipeline

```
file -> async/disk read -> decoder/resampler -> cache pages/SPSC ring -> RT clip node
```

Use full-memory assets only for small, frequently random-access samples where the memory budget supports it. Stream long clips and large sample content. Playback ring content is tagged with asset version, stream generation, channel/layout, source frame range, and decode/resample configuration.

## Read-ahead

- Target horizon derives from worst observed storage/decode latency, simultaneous streams, seek rate, and memory budget—not a fixed magical chunk size.
- Read chunks large enough for efficient I/O and codec frames but small enough for seek cancellation and cache fairness. Benchmark 64 KiB–1 MiB ranges by backend/media.
- Schedule low-water before starvation; prioritize streams by time-to-empty, playback rate, and audibility.
- Coalesce adjacent reads and share immutable decoded pages for clips referencing the same asset/range.
- Apply backpressure at high-water; never grow queues without bound.

## Seeking, loops, edits

A seek increments stream generation, cancels/ignores old work, resets decoder state at an appropriate keyframe, decodes preroll if required, and fills the new horizon. RT consumes only matching generation. Loop wrap should be prefetched before the boundary; crossfade/de-click policy is timeline-defined. Edits invalidate overlapping pages by content-addressed/versioned keys rather than mutating in-use buffers.

Compressed formats require codec workers because decoding time and random access are unpredictable. For edit-intensive source material, a background transcode to an internal seekable PCM/cache format may improve determinism while retaining original media.

## Many streams and starvation

Avoid one OS thread per track. Use a bounded I/O/decode scheduler with priority by deadline and device concurrency. Measure queue delay, read latency distribution, decoder CPU, ring fill, cache hit rate, and starvation per stream. SSDs still exhibit stalls; network/removable media needs larger horizons and explicit offline behavior.

On starvation, clip node emits a bounded silence or short de-clicked transition, advances timeline, and reports exact missing range. Do not repeat stale audio silently unless that is an explicit user mode.

## Recording pipeline

```
device input -> RT copy into preallocated SPSC pages -> recording writer -> temp media file
                                                      -> finalize/index/atomic project reference
```

- Preallocate/touch recording pages before punch/record where practical.
- RT assigns absolute sample positions and take generation; writer preserves gaps/overruns explicitly.
- Writer uses large sequential writes, periodic durable checkpoints appropriate to risk, and a recoverable temp/journal format.
- Punch-in/out boundaries are sample accurate in RT; file creation/finalization is not RT.
- Simultaneous tracks may share interleaved capture pages or separate rings; choose based on device layout and write efficiency.
- Disk-full/permission/device-removal errors surface immediately. Preserve recoverable partial audio. Never claim a take is valid after ring overwrite.

## Cache ownership

The stream manager owns decoded page cache and eviction. RT pins pages/ring spans for a quantum without reference destruction. Eviction runs off-RT and cannot reclaim pinned pages. Cache budget is explicit and metrics expose resident/high-water/eviction values.

## Tests

Inject read latency, partial reads, codec errors, disk-full, seek storms, loop wraps, and writer stalls. Validate exact sample placement and generation rejection. Stress simultaneous playback and recording while UI/cache/save jobs run.

## References

- Ardour source, [disk I/O and sources](https://github.com/Ardour/ardour/tree/master/libs/ardour).
- Microsoft, [File buffering](https://learn.microsoft.com/en-us/windows/win32/fileio/file-buffering).

