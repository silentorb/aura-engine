# aura_render — design

## Summary

Rendering for both playback modes: deterministic sampling of `Sampler` implementations over a render interval, and streaming blocks of live voices ([playback modes](../../../design/playback-modes.md)).

## Requirements

- Define the `Sampler` trait (`at(time_secs) -> f32`).
- Sample over `RenderSpec` (sample rate, duration, channels) into PCM buffers.
- Define `BlockSource` (consecutive mono blocks; time only moves forward).
- Provide `StreamVoice` (one runtime graph instance + its `LiveNote` + voice-local clock) and `VoiceMixer` (a `BlockSource` that takes note-on, note-off, and all-off commands from a control thread through `VoiceSender` without blocking, caps polyphony at `MAX_VOICES` by releasing the oldest voice, and retires released voices once silent).
- Remain independent of Imp JSON loading and WAV encoding details where possible.

## Non-goals / avoided approaches

- Device playback (belong in `aura_io_cpal`)
- Imp translation (belong in `aura_integration`)

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Code: [`../../../../crates/aura_render`](../../../../crates/aura_render)
