# aura_imp — design

## Summary

Aura-specific Imp node libraries, registries, and JSON helpers. Graphs denote pure **Time → Sample** functions; translation and sampling live in `aura_integration`.

## Requirements

- Provide node libraries: `aura.time`, `aura.dsp`, `aura.envelope`, `aura.music`.
- Depend on `imp_core_types` and `imp_registry` (and related Imp crates as needed for flatten/JSON); do not own the audio sampler runtime.
- Music nodes prepare translate-time data for composition crates; they do not interpret per sample.
- `external_score` is a nullary `score` source bound at translate time (`ScoreSignal`). Graphs **reference** on-disk `notation` tips by tip NodeId; `aura_integration` resolves those ids through the Miro host index (content root) into an `external_score` proxy + binding. The Aura editor opens the same tip as a nullary `external_score` with embedded notation in the session model.

## Non-goals / avoided approaches

- Stock `imp_execution` for continuous audio buffers
- FunDSP lowering of Imp graphs
- Surface orchestration (CLI, WAV write)

## See also

- [`../../../design/imp-execution.md`](../../../design/imp-execution.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_imp`](../../../../crates/aura_imp)
