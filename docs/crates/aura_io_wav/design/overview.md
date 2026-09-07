# aura_io_wav — design

## Summary

32-bit float WAV write/read for offline validation and demo artifacts.

## Requirements

- Write mono `f32` PCM directly (no FunDSP `Wave` types).
- Support read-back verification for tests and tooling.
- Default demo/CLI output targets the gitignored `output/` directory.

## Non-goals / avoided approaches

- Real-time device playback
- Owning graph translation

## See also

- [`../../../design/product.md`](../../../design/product.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_io_wav`](../../../../crates/aura_io_wav)
