# Rendering, Freeze, Bounce, and Cache

## Render modes

| Operation | Output | Source remains editable | Typical graph |
|---|---|---:|---|
| Export/render | final file | yes | full selected output graph |
| Bounce | new clip/track | yes | selected routing scope |
| Stem render | multiple files | yes | multiple defined tap points |
| Freeze | managed cached audio replacing DSP | yes, after unfreeze | track/subgraph plus required context |
| Render cache | invisible reusable blocks | yes | eligible deterministic subgraphs/ranges |

Every artifact stores provenance and a validity key. Never treat a filename's existence as cache validity.

## Offline engine

Instantiate a separate prepared graph and runtime state. Do not process live stateful plugin instances concurrently or steal RT pools. Offline processing may use larger blocks and parallel scheduling, but boundary/event semantics must match live rendering. Larger blocks can change plugin behavior; offer fixed-live-block compatibility mode and test known plugins.

Render faster than realtime unless a plugin/hardware/external route declares realtime dependence, uses wall clock incorrectly, or fails offline validation. Such paths run realtime with an explicit reason. Cancellation and progress checks occur at bounded block boundaries.

## Determinism

Pin project revision, graph compiler version, sample rate/layout, tempo map, automation/events, plugin binary/version/state, random seeds, oversampling/quality mode, source content hashes, and render policy. Reset all DSP/plugin state and include required preroll. Bitwise repeatability is expected for controlled native DSP; third-party plugins may only meet tolerance or nondeterminism classification.

Never parallelize time slices of one stateful chain unless state transfer makes them independent. Parallelize independent graph branches, stems, or isolated ranges with complete state snapshots.

## Tails and latency

Render preroll for PDC/lookahead/history, trim/align using declared graph latency, and continue through finite tails. For unknown/infinite tails, use explicit maximum plus silence-threshold/hold policy, recorded in metadata. Bounce tap semantics (pre/post fader, sends, master) must be exact.

## Freeze boundary

Freeze selects a graph cut. Cache upstream audio plus event/latency/tail context; downstream live nodes remain active. A valid cut must preserve sidechain/send dependencies. Freezing a track that feeds a sidechain may cache its tapped signal too or retain the necessary upstream path. State the behavior rather than muting hidden dependencies.

## Validity key and invalidation

Include hashes/generations for:

- source audio/MIDI and relevant sample range;
- edits, warp/resample settings, fades, gain;
- automation/parameter values and plugin state/binary;
- routing up to the cache cut, sidechains/sends, channel format;
- tempo/sample rate/render quality and engine/DSP algorithm version;
- random seed/external dependency policy.

Invalidate only affected ranges/subgraphs when dependency analysis proves isolation; otherwise invalidate conservatively. Publication is atomic: incomplete/cancelled files never become valid. Eviction is non-RT and cannot remove pinned playback pages.

## Correctness tests

Compare live capture and offline/frozen output after latency alignment across block sizes, seeks, tails, automation boundaries, sidechains, and tempo maps. Use exact hashes for deterministic native nodes and documented numeric/audio tolerance for plugins. Unfreeze/refreeze must reproduce current project intent, not stale state.

## References

- Steinberg, [VST3 process modes](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical%2BDocumentation/Processing%2BSetup/Processing%2BSetup.html).
- CLAP, [render extension](https://github.com/free-audio/clap/blob/main/include/clap/ext/render.h).

