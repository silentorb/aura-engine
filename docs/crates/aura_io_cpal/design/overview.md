# aura_io_cpal — design

## Summary

CPAL bridge for real-time output in both playback modes ([playback modes](../../../design/playback-modes.md)). `PlaybackEngine` (deterministic transport) plays a translated `AudioSignal` on the default output device until stopped; callers may start at an arbitrary time via `start_at` and read the playhead via `position_secs`. `StreamingOutput` (streaming) is a persistent stream that pulls blocks from a `BlockSource`.

## Requirements

- `PlaybackEngine` renders the same frames as offline render. One render thread per playback owns the signal and renders it into a lock-free single-producer, single-consumer ring of stereo frames, `PREPARE_AHEAD_SECS` (2 s) ahead of the playhead. `start_at` fills the ring on the calling thread before opening the stream. The audio callback only copies frames out of the ring; when the ring is dry it plays silence and counts the missing frames (`underruns()`), and the playhead (`position_secs`) reports frames actually played ([plugin hosting](../../../design/plugin-hosting.md#transport-playback-render-ahead)).
- `stop()` closes the stream, then stops and joins the render thread.
- `StreamingOutput::open(source, sample_rate)` keeps the device stream open until dropped and runs independently of `PlaybackEngine` (both may sound at once). The audio callback never blocks and reuses a preallocated frame buffer.
- Expose `start(signal, sample_rate)`, `start_at(signal, sample_rate, start_secs)`, `stop()`, `is_playing()`, `position_secs()`, and `underruns()`.
- `start` is `start_at(..., 0.0)`.
- Fail with a clear error when no default output device exists (common under WSL/devcontainer); never panic.
- Do not block offline WAV workflows or require audio hardware in CI.
- Output is stereo frames (a mono graph is the same on both channels). A mono device gets their average; a device with more channels gets left/right on its first two channels and silence on the rest.

## Non-goals / avoided approaches

- Rendering the graph on the audio thread
- Requiring live audio hardware for CI or default demos
- A separate synthesis model per output; streaming differs from deterministic only through per-node streaming implementations chosen at translation
- Transport UI or playhead drawing in this crate (callers own UI; this crate exposes seek + position)

## Environment note

Device availability depends on the host. The API is present; absent devices are an environment limit, not a missing feature.

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_io_cpal`](../../../../crates/aura_io_cpal)
