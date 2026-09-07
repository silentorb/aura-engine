# aura_cli — design

## Summary

Thin graph-driven CLI over `aura_integration`: load graph, render duration, write WAV.

## Requirements

- Accept Imp graph path, output path, duration (seconds or measures), tempo, and sample rate.
- Default duration/tempo behavior suitable for demos; write to caller-specified paths (typically gitignored `output/`).
- Remain thin — business logic lives in `aura_integration` and domain crates.

## Non-goals / avoided approaches

- Embedding synthesis algorithms in the CLI crate
- Requiring live audio hardware for the default path

## See also

- [`../../../design/product.md`](../../../design/product.md)
- Code: [`../../../../crates/aura_cli`](../../../../crates/aura_cli)
