# aura_sample — design

## Summary

Sample primitives, sample rates, and timing math shared across Aura.

## Requirements

- Expose DASP `Sample` / `Signal` types used by render and I/O.
- Validate common PCM sample rates (`44100`, `48000`, …).
- Provide seconds↔frame helpers used by scheduler and render.

## Non-goals / avoided approaches

- Synthesis algorithms (belong in `aura_dsp`)
- File or device I/O

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Code: [`../../../../crates/aura_sample`](../../../../crates/aura_sample)
