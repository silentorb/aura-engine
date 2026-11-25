# aura_render — design

## Summary

Rendering for both playback modes: block signals that render ranges of frames, the adapters between them, sinks over them (offline rendering, `Renderer`), and streaming blocks of live voices ([execution model](../../../design/imp-execution.md), [playback modes](../../../design/playback-modes.md)).

## Requirements

- Define `Frame` (an integer frame index), `BLOCK_FRAMES` (256: the most frames a signal is asked for at once, and the plugin block size), and `frame_secs(frame, rate)`, the one way every signal computes a frame's time.
- Define the signal traits: `Signal` (`render(start, out)`: mono frames `start .. start + out.len()`) and `StereoSignal` (`render(start, left, right)`: planar stereo). Requests are at most `BLOCK_FRAMES`. Both are `Send` and take `&mut self`; state follows the mode's rule ([node state](../../../design/imp-execution.md#node-state-one-rule-per-mode)).
- Provide `Input` / `StereoInput` (a signal with its own block buffers, read by range) and `Constant`.
- Provide shared outputs: `Shared` / `SharedStereo` are cloneable handles to one source that render it once per requested range and give every clone that range's block.
- Define audio layouts: `Layout`, `AudioSignal` (mono or stereo), and the adapters `StereoChannel` (one channel of a stereo signal), `StereoPair` (two mono signals), `StereoToMono`, `MonoToStereo` ([audio layouts](../../../design/audio-layouts.md)).
- Render ranges of any length: `render_mono`, `Renderer` (block buffers held once, stereo frames or interleaved output, never allocates after construction), `sample_offline` / `sample_offline_audio` over `RenderSpec` (sample rate, duration, channels), and `render_range` for tests and tooling.
- Define `BlockSource` (consecutive blocks of stereo frames; time only moves forward).
- Provide `StreamVoice` (one runtime graph instance + its `LiveNote` + voice-local clock, rendering consecutive blocks from frame 0) and `VoiceMixer` (a `BlockSource` that takes note-on, note-off, and all-off commands from a control thread through `VoiceSender` without blocking, caps polyphony at `MAX_VOICES` by releasing the oldest voice, and retires released voices once silent). The mixer never allocates or frees on the audio thread: voices sit in `MAX_SLOTS` preallocated slots (the oldest releasing voice is cut when all are taken), and finished voices go back to the control thread, which drops them on its next send or `VoiceSender::reclaim`.
- Remain independent of Imp JSON loading and WAV encoding details where possible.

## Non-goals / avoided approaches

- Per-sample `at(t)` evaluation as the runtime form
- Real-time reader modes or prepare hooks on signals (transport render-ahead belongs in `aura_io_cpal`)
- Device playback (belongs in `aura_io_cpal`)
- Imp translation (belongs in `aura_integration`)

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Code: [`../../../../crates/aura_render`](../../../../crates/aura_render)
