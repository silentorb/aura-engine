# aura_instrumentation — design

## Summary

Instrument trait, per-note sampling, envelopes, and mix-down for offline render.

## Requirements

- Sample each note as a distinct instrument instance by default (per-note params and envelopes).
- Support future-aware sampling via `ScheduleContext` when available.
- Keep offline output deterministic.

## Non-goals / avoided approaches

- Imp node libraries (belong in `aura_imp`)
- Graph translation orchestration (belong in `aura_integration`)

## See also

- [`../../../design/notation-synthesis.md`](../../../design/notation-synthesis.md)
- Code: [`../../../../crates/aura_instrumentation`](../../../../crates/aura_instrumentation)
