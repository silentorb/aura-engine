# aura_imp — design

## Summary

Aura-specific Imp node libraries, registries, and JSON helpers. Graphs denote deterministic functions of frame (mono samples or stereo frames); translation and rendering live in `aura_integration`. Audio signal types and the `Audio` constraint: [audio layouts](../../../design/audio-layouts.md).

## Requirements

- Provide node libraries: `aura.time`, `aura.dsp`, `aura.envelope`, `aura.music`, `aura.plugin` (`plugin_instrument`, `plugin_effect`: plugin id, base64 state, and JSON params literals, no plugin-specific ports; see [plugin hosting](../../../design/plugin-hosting.md)).
- Depend on `imp_core_types` and `imp_registry` (and related Imp crates as needed for flatten/JSON); do not own the audio rendering runtime.
- Music nodes prepare translate-time data for composition crates; they do not interpret while rendering.
- Every node type has a deterministic implementation and may have a streaming one; node types from extensions follow the same rule ([playback modes](../../../design/playback-modes.md)).
- On-disk `notation` tips are referenced by tip NodeId from Imp graphs. `aura_integration` resolves those remote NodeIds through the Miro host index (content root) into a `score` port view + `ScoreSignal` binding—no Imp proxy type.
- Expose Aura Imp catalogs and Imp-graph CPAL playback to Imp Lab through the Aura pack's extension libraries (`aura_imp_lab_catalogs` / `aura_imp_lab_playback` cdylibs) that register via Lab’s extension ABI—not by Lab depending on Aura crates. The pack configuration is owned by [aura-editor](https://github.com/silentorb/aura-editor); see [`lab-pack/README.md`](../../../../lab-pack/README.md).

## Non-goals / avoided approaches

- Stock `imp_execution` for continuous audio buffers
- FunDSP lowering of Imp graphs
- Surface orchestration (CLI, WAV write)
- Putting Aura catalogs inside `imp-ts` or requiring Imp Lab to path-depend on Aura packages

## See also

- [`../../../design/imp-execution.md`](../../../design/imp-execution.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_imp`](../../../../crates/aura_imp)
