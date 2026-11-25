# aura_plugin_host — design

## Summary

Protocol-neutral hosting of third-party audio plugins in Aura's playback modes ([plugin hosting](../../../design/plugin-hosting.md)).

## Requirements

- Define the backend seam: `Processor` (an activated instance: layout, latency, tail, block `process` with block-relative note `Event`s), `ProcessorFactory` (creates instances of one plugin with stored state applied), `PluginResolver` (plugin id + state → factory), and `PluginHostError`.
- `LayoutStage` adapts any processor to a mono or stereo layout ([plugin hosting](../../../design/plugin-hosting.md)). `RenderCache` retains one or two channels and renders as a `Signal` (a stereo cache reads as the average of its channels) or `StereoSignal` (a mono cache on both channels); `PluginStage` (streaming) runs in stereo.
- Define plugin configuration from node data: `PluginSettings` (state blob, then `ParamSetting`s that override it) passed to `PluginResolver::resolve`, and `ParamDescription` / `ParamFlags` returned by `ProcessorFactory::params`. Parameter values are plain (the plugin's units).
- `NoteEventSource` turns a `ScoreSignal` into note events at exact frames, per block window, repeating looping scores without end; note-offs precede note-ons at equal frames; note ids are unique per note instance.
- `RenderCache` is the deterministic implementation: a block signal that renders the plugin forward from frame 0 on its own `BLOCK_FRAMES` grid into retained, frame-aligned output and answers any range from it, with latency compensation, tail handling, and an `EventSource` of a score, one live note (the streaming per-voice fallback), or none (an effect). Its audio input is a deterministic signal rendered over each plugin block. It meets the deterministic contract (any range, any order, identical frames) within a translation ([node state](../../../design/imp-execution.md#node-state-one-rule-per-mode)).
- `PluginStage` is the streaming shared instance: a `BlockSource` driven by `StageCommand`s through a non-blocking `StageSender`, optionally processing another `BlockSource` as its input. No allocation on the audio thread.
- Process in blocks of at most `BLOCK_FRAMES` (`aura_render::BLOCK_FRAMES`).
- Depend only on `aura_render`, `aura_sample`, `aura_sequencing`: no protocol crates, no Imp.

## Non-goals / avoided approaches

- Protocol code (belongs in backends such as `aura_clap`)
- Imp translation (belongs in `aura_integration`)
- Interpreting plugin state

## See also

- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_plugin_host`](../../../../crates/aura_plugin_host)
