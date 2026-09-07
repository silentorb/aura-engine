# Background

Cross-cutting context that informed Aura’s design. Prefer linking from design docs rather than duplicating chronology.

## Workspace growth

Aura spans composition, synthesis, scheduling, and I/O. A single crate with modules would couple concerns and blur dependency boundaries as the codebase grows. Small crates under `crates/` keep ownership explicit.

## Audio paths

Aura needs both offline rendering (disk) and eventual real-time playback, with shared synthesis between paths and sane Rust-ecosystem integration (DASP, CPAL).

## Dev environment

Primary development is WSL via **aura-workbench** (Dev Container + Compose): separate containers for the engine and editor. Real-time audio output is unreliable in this setup. Offline WAV validation is the practical first path; live CPAL remains deferred until the environment supports it.

## Multi-repo layout

Aura splits across kebab-case repos so orchestration stays out of the product tree:

| Repo | Role |
|------|------|
| `aura-workbench` | Primary workspace; Compose/Dev Container; mounts siblings |
| `aura-engine` | This product (Rust crates, docs, demos) |
| `aura-editor` | Editor UI (separate stack; placeholder at split time) |

Imp siblings remain path dependencies under `.mnt/` inside the engine container. Motivation: keep engine cloneable as product source while the workbench owns how developers attach tools and sibling checkouts.

## Declarative graphs via Imp

Aura needs a portable declarative DAG for synthesis. [Imp](https://github.com/silentorb/imp-spec) provides `Graph`, node libraries, and registries. Stock `imp_execution` evaluates collection/path graphs into tabular rows — the wrong model for continuous audio buffers.

imp-rust is bind-mounted at `.mnt/imp-rust` (host `~/dev/imp-rust`) so Cargo path deps pick up sibling changes without rebuilding the image.

## Seekable sampling vs sequential DSP

Aura targets explicit time and seekable pure `f(t)` sampling. Sequential `tick` DSP (phase accumulators, filter history between samples) is deterministic when run in order but is not a pure function of time — seeking requires simulating prior samples. That conflict is why FunDSP-style tick units are avoided; see design non-goals in [`../design/imp-execution.md`](../design/imp-execution.md) and [`../crates/aura_dsp/design/overview.md`](../crates/aura_dsp/design/overview.md).

## Domain separation

Notation types, procedural generators, temporal scheduling, and per-note instrument sampling are separate concerns so the notation→audio pipeline can evolve without collapsing into one module. Motivation for that split: clear *what* → *when* → *how* → *wiring*.
