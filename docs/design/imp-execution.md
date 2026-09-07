# Imp graph execution model

Declarative requirements for how Aura evaluates Imp graphs for audio synthesis.

Motivation: [`../scoping/background.md`](../scoping/background.md). Related: [`../crates/aura_imp/design/overview.md`](../crates/aura_imp/design/overview.md), [`../crates/aura_integration/design/overview.md`](../crates/aura_integration/design/overview.md), [`../crates/aura_dsp/design/overview.md`](../crates/aura_dsp/design/overview.md).

## Time → Sample semantics

An Aura Imp graph denotes a pure function:

```text
f : Time → Sample
```

- **Time** — seconds from render start (`f64`).
- **Sample** — mono amplitude (`f32`).
- The **`time`** signal is wired explicitly in graphs (see demo JSON under [`demos/`](../../demos/)).

## Three layers

| Layer | Role |
|-------|------|
| Imp JSON graph | Syntax — nodes and edges describing function composition |
| `translate_graph` | Compiler — builds `Box<dyn Sampler>` once |
| `sample_graph` | Applicator — calls `f(t)` per frame over a duration |

Translation walks the DAG once and composes nested closures (`Sampler::at(t)`). Imp nodes are not interpreted per sample. The custom Aura sampler lives in `aura_integration`.

## Functional properties (v1)

- **Referential transparency** — `at(t)` depends only on `t` and graph parameters.
- **Seekability** — `at(t)` is valid without evaluating prior samples.
- **Composition** — node wiring corresponds to composing child samplers.

Seekability and purity are testable (`at(t)` twice is identical; sparse `t` matches a sequential pass).

## Translate-time vs sample-time

| Kind | When | Examples |
|------|------|----------|
| Translate-time | During `translate_graph`; captured in closures | `arpeggio` → `Score`; `epic_minor_progression` → `ChordProgression`; schedule tables |
| Sample-time | Inside `at(t)` | `sine(f, t)`, envelopes, note gates |

Node libraries in `aura_imp`: `aura.time`, `aura.dsp`, `aura.envelope`, `aura.music`. Translate hooks compose pure functions using `aura_dsp` and composition crates.

## Duration

Render duration is passed alongside the graph (`SampleSpec` / CLI `--duration`). Supported units: seconds (`10s`) and measures (`2m` with `--tempo`). Graph-owned duration is deferred.

## Non-goals / avoided approaches

- Stock `imp_execution` (tabular row evaluation) for continuous audio
- FunDSP-style sequential `tick` state for the primary sampler path
- Interpreting Imp nodes per sample instead of compiling once at translate time

## Deferred

- Higher-order graphs (graph produces graph)
- Buffer/block batching without changing `f(t)` semantics
- Real-time CPAL sampler path
