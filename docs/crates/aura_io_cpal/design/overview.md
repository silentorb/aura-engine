# aura_io_cpal — design

## Summary

CPAL bridge for real-time output in both playback modes ([playback modes](../../../design/playback-modes.md)). `PlaybackEngine` (deterministic transport) plays a pure `Sampler::at(t)` on the default output device until stopped; callers may start at an arbitrary time via `start_at` and read the shared sample clock via `position_secs`. `StreamingOutput` (streaming) is a persistent stream that pulls blocks from a `BlockSource`.

## Requirements

- `PlaybackEngine` samples the same pure `Sampler::at(t)` functions as offline render. Before opening the stream, `start_at` calls `Sampler::prepare(start, PREPARE_AHEAD_SECS)` on the calling thread; the audio callback marks its thread a real-time reader (`aura_render::mark_realtime_reader`), so stateful samplers (plugin caches) never render on it. `underruns()` reports real-time reads that found nothing prepared ([plugin hosting](../../../design/plugin-hosting.md#transport-playback-render-ahead)). `StreamingOutput` does not mark its thread.
- `StreamingOutput::open(source, sample_rate)` keeps the device stream open until dropped and runs independently of `PlaybackEngine` (both may sound at once). The audio callback never blocks and reuses a preallocated mono buffer.
- Expose `start(sampler, sample_rate)`, `start_at(sampler, sample_rate, start_secs)`, `stop()`, `is_playing()`, `position_secs()`, and `underruns()`.
- `start` is `start_at(..., 0.0)`.
- Fail with a clear error when no default output device exists (common under WSL/devcontainer); never panic.
- Do not block offline WAV workflows or require audio hardware in CI.
- Mono sampler output is duplicated across output channels.

## Non-goals / avoided approaches

- Requiring live audio hardware for CI or default demos
- A separate synthesis model per output; streaming differs from deterministic only through per-node streaming implementations chosen at translation
- Transport UI or playhead drawing in this crate (callers own UI; this crate exposes seek + position)

## Environment note

Device availability depends on the host. The API is present; absent devices are an environment limit, not a missing feature.

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_io_cpal`](../../../../crates/aura_io_cpal)
