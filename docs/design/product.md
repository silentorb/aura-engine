# Product design

Declarative product requirements for Aura. Motivation and goals: [`../scoping/product.md`](../scoping/product.md).

## Core domains

| Domain | Description |
|--------|-------------|
| Sequencing | Notation types (`aura_sequencing`); offline scheduling (`aura_scheduler`); live dispatch deferred |
| Composition | Procedural generators (`aura_composition`) |
| Instrumentation | Per-note sampling and mix-down (`aura_instrumentation`) |
| Synthesis | Imp graphs (`aura_imp`) and pure DSP (`aura_dsp`) |
| Integration | Graph translation and sampling (`aura_integration`) |
| Rendering | Offline WAV export via thin CLI and demo scripts |

## Rendering

- Load Imp JSON graphs; translate to `Sampler`; sample over a specified duration.
- Write 32-bit float WAV files to untracked `output/` via `aura_cli` or [`demos/render-all.sh`](../../demos/render-all.sh).
- Real-time playback via CPAL is planned but deferred.

## Integration requirements

1. Per-note instrument instances with per-note parameters and envelopes
2. Future-aware sampling during offline render via `ScheduleContext`
3. Deterministic, seedable offline output
4. Functional notation generation over stored sequences
5. Modular, nestable, library-organized patterns
6. Long-term: holistic song-as-function via reducers (target, not v1)

Detail: [`notation-synthesis.md`](notation-synthesis.md).

## Repository boundaries

- **aura-engine** (this repo) owns Rust crates, product docs, and demos.
- **aura-workbench** owns Dev Container / Compose orchestration and multi-repo mounts.
- **aura-editor** owns editor UI (separate from the engine crate tree).

Repos and projects use kebab-case; Rust crates use snake_case.

## Non-goals / avoided approaches

- Live CPAL playback until the dev environment supports it reliably
- Stock `imp_execution` for Aura’s primary sampler path
- FunDSP-style sequential `tick` DSP for the primary synthesis path
- Embedding workbench orchestration (Compose/Dev Container entrypoint) inside this engine repo

Background: [`../scoping/background.md`](../scoping/background.md).
