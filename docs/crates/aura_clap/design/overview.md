# aura_clap — design

## Summary

CLAP backend for [`aura_plugin_host`](../../aura_plugin_host/design/overview.md), built on `clack-host` ([plugin hosting](../../../design/plugin-hosting.md#clap-backend)).

## Requirements

- `ClapHost` finds `.clap` bundles in extra paths, `CLAP_PATH`, and the platform's standard locations; lists their plugins (`discover`, with load failures reported separately); and implements `PluginResolver`. Bundles load lazily and stay loaded. `add_entry` registers an already loaded entry (in-process plugins under test).
- `ClapFactory` creates instances: instantiate, load stored state (state extension required when state is given), request the render mode (offline for deterministic, real-time for streaming; a refusal is not an error), read audio-port layout, activate, read latency.
- `ClapProcessor` implements `Processor`: buffers for every audio port, note events as CLAP note-on/off on note port 0 channel 0, tail from the tail extension (unknown without it). Dropping it deactivates and destroys the instance on the main thread.
- All main-thread calls for all instances run on one process-wide CLAP main thread, which also services `request_callback`. Host logging goes to stderr.
- `aura_clap_fixture` (test-only, `publish = false`) provides deterministic CLAP plugins: `aura.fixture.sine` (polyphonic sine, 64-frame latency, 0.1 s release) and `aura.fixture.gain` (gain in its state). It builds as a `.clap` bundle (`cdylib`) and loads in-process in tests.

## Non-goals / avoided approaches

- One main thread per instance (breaks plugins that assume a single main thread)
- Mode handling (belongs in `aura_plugin_host`)
- Plugin GUIs (later phase)
- Out-of-process hosting (deferred)

## See also

- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_clap`](../../../../crates/aura_clap), [`../../../../crates/aura_clap_fixture`](../../../../crates/aura_clap_fixture)
