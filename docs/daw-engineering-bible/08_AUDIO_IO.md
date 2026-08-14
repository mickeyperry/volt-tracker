# Audio I/O

## Deadline relationship

For sample rate `R` and callback frames `N`, nominal period `D = N/R`:

| Format | Deadline | Calls/second |
|---|---:|---:|
| 48 kHz / 64 | 1.333 ms | 750 |
| 48 kHz / 128 | 2.667 ms | 375 |
| 96 kHz / 128 | 1.333 ms | 750 |

At 96 kHz / 128, wall-clock budget equals 48 kHz / 64 while many DSP algorithms do about twice the sample work per second. The complete required graph, backend copies/conversion, and scheduling overhead must finish before the effective deadline. Eight idle cores do not rescue a 1.5 ms serial path from a 1.333 ms deadline.

## Latency components

User-perceived latency is not just `N/R`: capture buffering + ADC/driver + application graph/PDC + render buffering + DAC/driver, with resamplers/safety offsets where present. Report device input/output/round-trip estimates separately from callback period. Measure loopback round trip where possible; driver reports are not ground truth.

Smaller buffers reduce nominal interaction latency but raise callback rate, scheduling overhead, and underrun sensitivity. Larger buffers increase tolerance/throughput but latency. Plugin algorithmic latency is independent and compensated by PDC, not erased.

## Backend contract

- Support WASAPI on Windows, Core Audio on macOS, and PipeWire/JACK/ALSA strategy on Linux as product scope dictates; ASIO may be supported under its SDK/licensing/build constraints.
- Enumerate APIs and devices off RT. Device IDs must be persistent backend identifiers, not list indices.
- Negotiate supported sample formats/rates/channel layouts/periods. The driver may reject or override requests; display actual values.
- Callback receives non-owning spans and timing/clock observations; perform only bounded conversion and graph execution.
- Prefer event-driven/exclusive or professional low-latency modes when requested and supported; shared modes trade compatibility for added system mixing/resampling.

## Formats and layout

Use one canonical internal DSP format (typically planar `f32`) unless measured precision requirements justify `f64`. Convert integer/packed/interleaved device input to planar pools and back at I/O boundaries with saturation/dither policy. Avoid conversion when backend format already matches. Validate channel maps; channel count is not speaker identity.

## Clocks

The device clock drives callback cadence and the live engine sample counter. Wall clock is telemetry/synchronization input, never a replacement. Input and output devices may have distinct drifting clocks; aggregate-device/resampling logic must estimate drift and elastically reconcile it. Do not assume nominal sample rate equals measured rate.

## Input/output buffering

Capture data for quantum `n` must be available before descendants run. Output is delivered in backend-required order. Extra application buffering adds predictable latency and may be an explicit safety mode. Never silently add blocks to solve performance without updating latency reporting and monitoring behavior.

## Device changes and loss

1. Backend/control detects change outside callback.
2. Stop/transition stream according to API; RT uses bounded fallback.
3. Negotiate new actual format/period off-RT.
4. Reprepare format-dependent nodes, plugin setup, pools, resamplers, PDC, and stream readers.
5. Atomically activate prepared graph then restart with de-click/fade and clock discontinuity policy.

No callback performs enumeration, graph rebuild, plugin reset, or allocation. Preserve user selection; never trust a remembered enumeration index. VOLT measurements show ASIO drivers can ignore requested sizes and differ materially from WASAPI, so capability and latency are runtime facts.

## Underruns

Count backend xrun flags and internal deadline misses separately. Record callback start jitter, duration, effective deadline, device/API/actual period, graph generation, and critical path. Avoid expensive logging in callback; write compact telemetry.

## References

- Microsoft, [Low Latency Audio](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio).
- Microsoft, [WASAPI](https://learn.microsoft.com/en-us/windows/win32/coreaudio/wasapi).
- Apple, [Core Audio Overview](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/Introduction/Introduction.html).
- JACK, [API documentation](https://jackaudio.org/api/).
- PortAudio, [Buffering, Latency and Timing](https://github.com/PortAudio/portaudio/wiki/BufferingLatencyAndTimingImplementationGuidelines).

