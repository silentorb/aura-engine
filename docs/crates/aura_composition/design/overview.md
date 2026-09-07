# aura_composition — design

## Summary

Procedural generators that produce notation (`Score`, `ChordProgression`, and related types).

## Requirements

- Generators emit **one finite cycle** (typically measured in bars), not unbounded playback.
- Favor functional generation over destructive stored sequences.
- Depend on `aura_sequencing` only among Aura domain crates.

## Non-goals / avoided approaches

- Sample-time synthesis
- Imp node definitions (belong in `aura_imp`)
- Surface orchestration (belong in `aura_integration` / CLI)

## See also

- [`../../../design/notation-synthesis.md`](../../../design/notation-synthesis.md)
- Code: [`../../../../crates/aura_composition`](../../../../crates/aura_composition)
