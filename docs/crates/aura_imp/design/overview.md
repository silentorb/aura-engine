# aura_imp — design

## Summary

Aura-specific Imp node libraries, registries, and JSON helpers. Graphs denote pure **Time → Sample** functions; translation and sampling live in `aura_integration`.

## Requirements

- Provide node libraries: `aura.time`, `aura.dsp`, `aura.envelope`, `aura.music`.
- Depend on `imp_core_types` and `imp_registry` (and related Imp crates as needed for flatten/JSON); do not own the audio sampler runtime.
- Music nodes prepare translate-time data for composition crates; they do not interpret per sample.
- On-disk `notation` tips are referenced by tip NodeId from Imp graphs. `aura_integration` resolves those remote NodeIds through the Miro host index (content root) into a `score` port view + `ScoreSignal` binding—no Imp proxy type. The Aura editor opens the same tip as a notation document for the piano roll.

## Non-goals / avoided approaches

- Stock `imp_execution` for continuous audio buffers
- FunDSP lowering of Imp graphs
- Surface orchestration (CLI, WAV write)

## See also

- [`../../../design/imp-execution.md`](../../../design/imp-execution.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_imp`](../../../../crates/aura_imp)
