# aura_plugin_host — design

## Summary

Protocol-neutral hosting of third-party audio plugins in Aura's playback modes ([plugin hosting](../../../design/plugin-hosting.md)).

## Requirements

- Define the backend seam: `Processor` (an activated instance: layout, latency, tail, block `process` with block-relative note `Event`s), `ProcessorFactory` (creates instances of one plugin with stored state applied), `PluginResolver` (plugin id + state → factory), and `PluginHostError`.
- `LayoutStage` adapts any processor to a mono or stereo layout ([plugin hosting](../../../design/plugin-hosting.md)). `RenderCache` stores one or two interleaved channels and reads as a `Sampler` or `StereoSampler`; `PluginStage` (streaming) runs in stereo.
- Define plugin configuration from node data: `PluginSettings` (state blob, then `ParamSetting`s that override it) passed to `PluginResolver::resolve`, and `ParamDescription` / `ParamFlags` returned by `ProcessorFactory::params`. Parameter values are plain (the plugin's units).
- `NoteEventSource` turns a `ScoreSignal` into note events at exact frames, per block window, repeating looping scores without end; note-offs precede note-ons at equal frames; note ids are unique per note instance.
- `RenderCache` is the deterministic implementation: a `Sampler` rendering forward into a retained buffer, with latency compensation, tail handling, interpolation off the frame grid, and an `EventSource` of a score, one live note (the streaming per-voice fallback), or none (an effect). Clones share one cache. Output is published through lock-free append-only chunks; reads from a real-time reader never render (silence and an underrun count instead); `prepare` renders ahead and starts one render-ahead worker per cache that exits when the cache finishes or is dropped.
- `PluginStage` is the streaming shared instance: a `BlockSource` driven by `StageCommand`s through a non-blocking `StageSender`, optionally processing another `BlockSource` as its input. No allocation on the audio thread.
- Process in blocks of at most `BLOCK_FRAMES`.
- Depend only on `aura_render`, `aura_sample`, `aura_sequencing`: no protocol crates, no Imp.

## Non-goals / avoided approaches

- Protocol code (belongs in backends such as `aura_clap`)
- Imp translation (belongs in `aura_integration`)
- Interpreting plugin state

## See also

- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_plugin_host`](../../../../crates/aura_plugin_host)
