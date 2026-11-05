# aura_io_cpal — design

## Summary

CPAL bridge for real-time playback. `PlaybackEngine` streams a pure `Sampler::at(t)` from `t = 0` on the default output device until stopped.

## Requirements

- Sample the same pure `Sampler::at(t)` functions as offline render.
- Expose `start(sampler, sample_rate)`, `stop()`, and `is_playing()`.
- Fail with a clear error when no default output device exists (common under WSL/devcontainer); never panic.
- Do not block offline WAV workflows or require audio hardware in CI.
- Mono sampler output is duplicated across output channels.

## Non-goals / avoided approaches

- Requiring live audio hardware for CI or default demos
- A separate synthesis model for real-time vs offline
- Transport UI, playhead, or seek APIs in this crate (callers own start time; this crate always begins at `t = 0`)

## Environment note

Device availability depends on the host. The API is present; absent devices are an environment limit, not a missing feature.

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_io_cpal`](../../../../crates/aura_io_cpal)
