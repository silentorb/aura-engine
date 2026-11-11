# aura_imp — design

## Summary

Aura-specific Imp node libraries, registries, and JSON helpers. Graphs denote pure **Time → Sample** functions; translation and sampling live in `aura_integration`.

## Requirements

- Provide node libraries: `aura.time`, `aura.dsp`, `aura.envelope`, `aura.music`.
- Depend on `imp_core_types` and `imp_registry` (and related Imp crates as needed for flatten/JSON); do not own the audio sampler runtime.
- Music nodes prepare translate-time data for composition crates; they do not interpret per sample.
- `notation_score` declares a string `path` input and a `score` output; path resolution and Aura notation file loading happen in `aura_integration` at translate time. This is how graphs **reference** notation storage on disk. Separately, the Aura editor may **open** a notation file as a nullary `external_score` node with embedded notation in the session model (same on-disk schema; different in-memory entry point).

## Non-goals / avoided approaches

- Stock `imp_execution` for continuous audio buffers
- FunDSP lowering of Imp graphs
- Surface orchestration (CLI, WAV write)

## See also

- [`../../../design/imp-execution.md`](../../../design/imp-execution.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_imp`](../../../../crates/aura_imp)
