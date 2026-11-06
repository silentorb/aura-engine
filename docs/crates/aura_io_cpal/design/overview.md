# aura_io_cpal — design

## Summary

CPAL bridge for real-time playback. `PlaybackEngine` streams a pure `Sampler::at(t)` on the default output device until stopped. Callers may start at an arbitrary time via `start_at` and read the shared sample clock via `position_secs`.

## Requirements

- Sample the same pure `Sampler::at(t)` functions as offline render.
- Expose `start(sampler, sample_rate)`, `start_at(sampler, sample_rate, start_secs)`, `stop()`, `is_playing()`, and `position_secs()`.
- `start` is `start_at(..., 0.0)`.
- Fail with a clear error when no default output device exists (common under WSL/devcontainer); never panic.
- Do not block offline WAV workflows or require audio hardware in CI.
- Mono sampler output is duplicated across output channels.

## Non-goals / avoided approaches

- Requiring live audio hardware for CI or default demos
- A separate synthesis model for real-time vs offline
- Transport UI or playhead drawing in this crate (callers own UI; this crate exposes seek + position)

## Environment note

Device availability depends on the host. The API is present; absent devices are an environment limit, not a missing feature.

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_io_cpal`](../../../../crates/aura_io_cpal)
