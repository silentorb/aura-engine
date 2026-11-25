# Block processing: motivation

Why Aura renders signals a block of frames at a time instead of one sample at a time, and why nodes may now keep state. Design: [`../design/imp-execution.md`](../design/imp-execution.md).

## What the per-sample model was for

Aura chose a pure, seekable `f(t)` model early ([background](background.md#seekable-sampling-vs-sequential-dsp)): a translated graph was a tree of nested `Sampler::at(t)` closures, asked for one sample at a time, with no state between calls. That bought the properties Aura cares most about: deterministic output, any time in any order, translation once. Block batching was listed as deferred, "without changing `f(t)` semantics".

The semantics held up. The execution model did not.

## The workarounds it caused

Every place Aura grew past pure oscillators and envelopes worked around the per-sample, stateless shape:

- **Plugins.** A plugin is block-based and stateful by nature. To answer `at(t)`, the plugin cache rendered forward into a lock-free, append-only store of atomics (hours of it), then looked samples up with linear interpolation off the frame grid. The plugin's audio input was pulled into each of its blocks one `at()` call per frame.
- **Transport playback.** The audio callback walked the whole closure tree once per frame. A plugin read could block there, so each cache had a "real-time reader" mode (a thread-local flag), its own render-ahead worker thread, and an underrun counter; `Sampler` grew `prepare` and `underruns` for it.
- **Fan-out.** A node feeding two consumers was translated twice — two copies of the subgraph, computed twice per sample, and two plugin instances if a plugin sat there.
- **Stereo.** Per-frame stereo reads computed both channels, so adapters (`ChannelView`, `StereoPair`, `SharedSampler`, a per-channel read method, remembered dual-mono inputs) existed to avoid computing the unneeded one.
- **DSP.** No node could keep state, so `highpass_noise` is `noise(t) − noise(t − dt)`, an exponential sweep uses a closed-form phase integral, and recursive filters and phase accumulators were an open question.

These share one cause, and they concentrate where Aura is growing: plugins, stereo, live playback.

## What changes

Signals render a requested **range of frames** into buffers. Each node output is built once and shared by everything that reads it. Time is indexed by frame; the `time` signal is still seconds, computed per frame exactly as before, so output is bit-identical.

## State: one rule per mode

Nodes may keep state now. The rule depends on the mode:

- **Deterministic:** state is **nonessential**. A node must still render any range, independent of its state, and return the same result. State may only make that faster (a cursor into a note schedule, a cache). Determinism and seek-anywhere stay as they were, with no pre-roll.
- **Streaming:** state may carry **history**. Streaming only ever moves forward, so a true recursive filter or a phase accumulator is fine there — as a node type's streaming implementation.

This fits the existing implementation matrix (mono/stereo × deterministic/streaming): a node type that wants a history-dependent algorithm provides it as its streaming implementation, and keeps a history-free deterministic one.

Plugins are the one exception in deterministic mode: their state cannot be snapshotted. The plugin node renders forward from frame 0 on its own block grid and keeps what it rendered, so from outside it still meets the rule; only the first visit to a far position pays for rendering up to it, as before.

## Alternatives considered

- **Blocks for speed only, every node stays pure, plugins stay a special frozen region.** Fixes fan-out and dispatch cost, but leaves the plugin exception as a template for the next stateful feature and keeps stateful DSP off-limits everywhere.
- **Stateful deterministic nodes rendered from frame 0 (seek = pre-roll).** Makes plugins unexceptional, but every stateful node would make seeking cost proportional to position. Rejected in favour of keeping deterministic state nonessential.
- **DAW-style state everywhere, seeking just continues.** Simplest at run time, but transport playback would stop matching WAV export, breaking the deterministic reference. Rejected.

## Follow-ups this enables

- Stateful streaming implementations: real filters, phase-accumulating oscillators.
- Exact deterministic replacements for the stateless approximations (only where they stay bit-for-bit seekable, or as a deliberate, separately reviewed output change).
- Control-rate signals (one value per block).
- Note-off on its exact frame inside a block in streaming.
