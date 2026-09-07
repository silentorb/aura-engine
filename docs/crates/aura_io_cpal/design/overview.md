# aura_io_cpal — design

## Summary

CPAL bridge for real-time playback. Currently a compile-time stub with sample-format conversion support; active playback is deferred.

## Requirements

- Remain present in the workspace so DASP↔CPAL sample conversion and dependencies stay wired.
- Future playback must sample the same pure `Sampler::at(t)` functions as offline render.
- Do not block offline WAV workflows.

## Non-goals / avoided approaches

- Requiring live audio hardware for CI or default demos
- A separate synthesis model for real-time vs offline

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_io_cpal`](../../../../crates/aura_io_cpal)
