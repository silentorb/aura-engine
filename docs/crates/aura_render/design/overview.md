# aura_render — design

## Summary

Offline sampling of `Sampler` implementations over a render interval.

## Requirements

- Define the `Sampler` trait (`at(time_secs) -> f32`).
- Sample over `RenderSpec` (sample rate, duration, channels) into PCM buffers.
- Remain independent of Imp JSON loading and WAV encoding details where possible.

## Non-goals / avoided approaches

- Device playback (belong in `aura_io_cpal`)
- Imp translation (belong in `aura_integration`)

## See also

- [`../../../design/architecture.md`](../../../design/architecture.md)
- Code: [`../../../../crates/aura_render`](../../../../crates/aura_render)
