# Imp graph execution model

Declarative requirements for how Aura evaluates Imp graphs for audio synthesis.

Motivation: [`../scoping/background.md`](../scoping/background.md), [`../scoping/block-processing.md`](../scoping/block-processing.md). Related: [`../crates/aura_imp/design/overview.md`](../crates/aura_imp/design/overview.md), [`../crates/aura_integration/design/overview.md`](../crates/aura_integration/design/overview.md), [`../crates/aura_render/design/overview.md`](../crates/aura_render/design/overview.md), [`../crates/aura_dsp/design/overview.md`](../crates/aura_dsp/design/overview.md).

## Frame → Sample semantics

An Aura Imp graph denotes a deterministic function of frame index:

```text
f : Frame → Sample
```

- **Frame** — an integer frame index from render start, at the render's sample rate. Its time in seconds is `frame / rate` (`aura_render::frame_secs`), computed the same way everywhere, so a frame has one time throughout a graph.
- **Sample** — a mono amplitude (`f32`) or a stereo frame (`[left, right]`), by the output's [audio layout](audio-layouts.md).
- The **`time`** signal is wired explicitly in graphs (see demo JSON under [`demos/`](../../demos/)); its value at a frame is that frame's time in seconds.

## Three layers

| Layer | Role |
|-------|------|
| Imp JSON graph | Syntax — nodes and edges describing function composition |
| `translate_graph` | Compiler — builds the graph's signals once (`AudioSignal`) |
| `sample_graph` / sinks | Applicator — render ranges of frames (WAV export, transport, voices) |

Translation walks the DAG once and builds a **signal** per node output: a `Signal` (mono) or `StereoSignal` that renders a requested range of frames into buffers (`render(start, out)`, at most `BLOCK_FRAMES` = 256 frames per call). A node's signal reads its inputs as ranges too. Each node type's code comes from its registered [implementation matrix](audio-layouts.md#implementation-matrix): one cell per layout × mode. Imp nodes are not interpreted while rendering. The custom Aura translator lives in `aura_integration`; the signal traits and sinks in `aura_render`.

## Shared outputs

- Each node output is built once per translation. An output read by more than one consumer (fan-out, both channel passes of a dual-mono node, several output ports of one node) is shared: consumers read clones of one handle (`Shared` / `SharedStereo`), which renders its source once per requested range.
- A node is never duplicated by translation. A plugin node read by several consumers is one plugin instance.

## Node state: one rule per mode

Signals may keep mutable state. What state may do depends on the mode the graph is translated for ([playback modes](playback-modes.md)):

| | Deterministic | Streaming |
|---|---|---|
| State | Nonessential: a cache or a shortcut only | May carry history (filter memory, oscillator phase) |
| Ranges requested | Any range, in any order | Consecutive blocks from frame 0, forward only |
| Contract | Rendering a range gives identical output whether the signal is fresh or used, whatever was rendered before | Output may depend on what was rendered before |

- **Deterministic contract.** For every deterministic signal, rendering frames `a..b` returns the same bits regardless of block boundaries, request order, earlier requests, or whether the signal is freshly translated. A deterministic implementation may use state only where it reproduces that output exactly (for example a cursor into a note schedule that is re-searched when a range does not follow the last one).
- **Seeking is free** in deterministic mode: no pre-roll, no reset. A graph renders from any frame as cheaply as from frame 0 (plugins excepted, below).
- **History belongs in streaming.** A node type whose best algorithm needs history (a recursive filter, a phase accumulator) provides it as its streaming implementation and keeps a history-free deterministic implementation. Approximations of history-dependent DSP in deterministic mode (`highpass_noise` as `noise(t) − noise(t − dt)`, closed-form sweep phase) are deterministic implementations, not workarounds to keep.
- **Look-around.** A deterministic signal may request its input over a different range than the one it renders (earlier or later frames), since its input is deterministic too.
- **Plugins are the bounded exception.** A plugin's state is essential and cannot be snapshotted. Its deterministic implementation renders the plugin forward from frame 0 on its own block grid and retains the output, then answers any range from what it kept ([plugin hosting](plugin-hosting.md)). It meets the deterministic contract within a translation; the first request far into a piece renders the plugin up to there.
- The contract is tested for every registered node type (`translate/contract_tests.rs`): one long range against uneven chunks, shuffled ranges, a used signal rendering again, and a fresh signal per range.

## Translation modes

Translation targets one of two modes ([`playback-modes.md`](playback-modes.md)). **Deterministic** is the default and has the properties above. **Streaming** translates a voice graph once per live note; frames only move forward and are voice-local (the note's onset is frame 0). Every node type has a deterministic implementation and may have a streaming one; streaming builds the streaming implementation when it exists and the deterministic one otherwise. The runtime form is the same block signals in both modes, and nodes never check the mode while rendering.

## Translate-time vs render-time

| Kind | When | Examples |
|------|------|----------|
| Translate-time | During `translate_graph`; captured in signals | `arpeggio` → `Score`; `epic_minor_progression` → `ChordProgression`; schedule tables |
| Render-time | Inside `render`, per frame of the block | `sine(f, t)`, envelopes, note gates |

Node libraries in `aura_imp`: `aura.time`, `aura.dsp`, `aura.envelope`, `aura.music`. Translate hooks compose functions from `aura_dsp` and composition crates.

## Duration

Render duration is passed alongside the graph (`SampleSpec` / CLI `--duration`). Supported units: seconds (`10s`) and measures (`2m` with `--tempo`). Graph-owned duration is deferred.

## Non-goals / avoided approaches

- Stock `imp_execution` (tabular row evaluation) for continuous audio
- Per-sample evaluation (`at(t)` per frame through nested closures) as the runtime form
- History-carrying node state in deterministic mode, and seeking by pre-roll for Aura's own nodes
- DAW-style state that lets transport playback diverge from offline rendering
- Translating a node more than once for several consumers
- Mode flags read while rendering; the mode is chosen once, at translation
- Interpreting Imp nodes while rendering instead of building signals once at translate time

## Deferred

- Higher-order graphs (graph produces graph)
- Control-rate signals (one value per block)
- Stateful streaming implementations (recursive filters, phase accumulators)
