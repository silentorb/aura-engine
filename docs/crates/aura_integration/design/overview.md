# aura_integration — design

## Summary

Surface integration library: load Imp graphs, `translate_graph` to `Sampler`, sample PCM, write WAV helpers.

## Requirements

- Implement Aura’s custom Time → Sample translator (not stock `imp_execution`).
- Compose `Sampler` closures once at translate time; apply `f(t)` per frame for sampling.
- Translate for both playback modes ([playback modes](../../../design/playback-modes.md)): every node type has a deterministic implementation and may have a streaming one, chosen once at translation (streaming falls back to the deterministic implementation and never fails because of the mode). `StreamingProgram` prepares a voice graph once and instantiates it per live note.
- Declare direct dependencies on every domain crate it wires — no hidden transitive orchestration.
- Expose duration/`SampleSpec` parsing used by the CLI.
- Host Miro sessions through **`imp_mir`**: Aura JSON kind mapping and `pull_graph` only — do not own Mir fabric.

## Non-goals / avoided approaches

- Thick CLI UX beyond library helpers (belong in `aura_cli`)
- Sequential tick DSP as a separate backend (streaming shares the `Sampler` runtime form)
- Reimplementing Mir store registry / sync wire-up inside Aura

## See also

- [`../../../design/imp-execution.md`](../../../design/imp-execution.md)
- [`../../../design/notation-synthesis.md`](../../../design/notation-synthesis.md)
- [`../../../design/miro.md`](../../../design/miro.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_integration`](../../../../crates/aura_integration)
