# Plugin Host Architecture

## Trust boundary

VST3, CLAP, and AU plugins are third-party code. They may allocate, lock, spawn threads, use denormals, misreport latency/tails, retain pointers, crash, hang, or require GUI/main-thread behavior. The host obeys each format contract but does not infer safety from successful scanning.

## Subcomponents

| Component | Domain | Responsibility |
|---|---|---|
| Scanner/probe | worker process preferred | discover binaries/classes, metadata, validation, cache |
| Instance manager | control/plugin management | module lifetime, creation, bus negotiation, prepare/activate/reset/destroy |
| Process adapter | DSP node/RT | prebuilt buffer/event views, context, `process` only |
| State service | control/save | capture/restore versioned opaque state under safe lifecycle |
| GUI bridge | UI/main | create/attach/resize/show/hide editor, parameter gestures |
| Sandbox supervisor (optional) | non-RT control | process lifecycle, shared audio/event transport, crash recovery |

## Lifecycle state machine

```
discovered -> module loaded -> instance created/initialized
 -> buses configured -> processing setup -> active -> processing
 -> stop processing -> inactive -> state/teardown -> destroyed
```

Transitions occur off RT except `process` and format-permitted bounded event exchange. Never race activation, bus configuration, state restore, GUI destruction, or instance destruction with processing. Sample-rate/max-block/bus changes require a prepared transition or stopped stream.

For VST3, follow component/controller separation; provide a valid host context even where the spec permits null because real plugins may assume one. Hold the module mapping longer than every object whose destructor/code lives in it. VOLT's current Windows host deliberately avoids unsafe DLL unload/teardown patterns verified to crash plugins; retain that behavior unless cross-plugin lifecycle tests prove a replacement.

## Process contract

- Prepare pointer arrays, bus descriptors, channel layouts, event/parameter containers, transport/process context, silence flags, and scratch before RT.
- Split a device quantum at sample-accurate event boundaries only when required by format/plugin behavior and benchmark the overhead; otherwise pass timestamped events with offsets.
- Never call one instance concurrently. Do not assume two instances from one module are mutually thread-safe; maintain a compatibility capability/quirk database when evidence requires serialization.
- Handle zero-frame/flush calls only as specified by the format.
- Bound output events. Validate sample offsets, IDs, channel counts, and plugin-written sizes.

## Parameters and automation

Separate normalized parameter identity/value from UI labels. Control gestures enter a timestamped command/event pipeline. Preserve stable ordering for multiple changes at one sample. VST3 parameter queues and CLAP events carry sample offsets; AU scheduling depends on the selected AU API. UI polling is never automation timing.

Host-to-plugin parameter changes, plugin-to-host gestures, state recall, modulation, and automation writeback are distinct flows. Prevent feedback loops and mark origin/generation.

## Buses, sidechains, and events

Negotiate main/aux audio buses and speaker/channel layouts before activation. Compile each active bus into graph ports. Sidechain activation changes graph dependencies. Instruments normally consume note/event input and produce audio; effects may also consume/produce events. Supply valid buffers and silence according to each specification, including disconnected/inactive-bus nuances.

## Latency and tails

Capture reported latency during prepare and respond to runtime change notifications by scheduling graph/PDC rebuild—never rebuild in callback. Until swap, use defined old-latency behavior. Tail declarations control post-input processing and freeze/bounce duration; cap pathological/infinite tails by explicit render policy. Measure known plugin quirks rather than silently overriding reports.

## State

Opaque state serialization may allocate/block and must occur off RT. If a format does not guarantee concurrent state access, suspend/clone the instance or maintain a control-side shadow; do not call it from autosave against live processing. Store plugin format, class ID, vendor/version, binary fingerprint/path hint, state schema envelope, bus layout, and host parameters. Missing plugins load as recoverable placeholders preserving state/routing.

## GUI

Plugin GUI is separate from audio processing. Create and pump it on the platform-required UI/main thread. GUI close SHOULD hide/reuse where plugins cannot safely recreate views; destroy only under controlled teardown. Parameter communication uses bounded queues/atomics. A stalled editor cannot block processing.

VOLT-specific Windows evidence: VST editor windows must be created on the main thread; VST2 editors may require periodic idle calls; teardown ordering and re-entrant resize need guards. Keep these as host compatibility tests.

## Crash and sandbox policy

In-process hosting has lowest transport latency but cannot reliably recover from access violations or hangs. Optional process sandboxing provides fault containment at costs: at least shared-memory synchronization, possible extra buffering/latency, GUI embedding complexity, and higher memory. Prefer one plugin or bounded group per process based on isolation/performance policy. A sandbox deadline miss returns a predeclared bypass/silence fallback and marks the instance unhealthy; never block callback waiting for IPC.

Scanning MUST use a disposable worker process with timeout and cache crash/hang results. Do not load unknown plugin binaries in the main app merely to inspect metadata.

## Format notes

- **VST3**: component/controller lifecycle, `setupProcessing`, bus arrangements, sample-offset parameter/event queues, latency-change restart flags.
- **CLAP**: explicit main/audio thread contracts, extensions for latency, tail, GUI, thread pool, timers; validate extension availability/version.
- **AU/AUv3**: follow Apple render-resource allocation and render-block thread restrictions; UI and component lifetime remain separate.

## References

- Steinberg, [VST3 API documentation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical%2BDocumentation/API%2BDocumentation/Index.html).
- Steinberg, [VST3 processing FAQ](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Processing.html).
- CLAP, [official specification repository](https://github.com/free-audio/clap).
- Apple, [Audio Unit Hosting Guide](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/AudioUnitHostingGuide_iOS/Introduction/Introduction.html).

