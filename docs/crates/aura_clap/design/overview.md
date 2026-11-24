# aura_clap — design

## Summary

CLAP backend for [`aura_plugin_host`](../../aura_plugin_host/design/overview.md), built on `clack-host` ([plugin hosting](../../../design/plugin-hosting.md#clap-backend)).

## Requirements

- `ClapHost` finds `.clap` bundles in extra paths, `CLAP_PATH`, and the platform's standard locations; lists their plugins (`discover`, with load failures reported separately); and implements `PluginResolver`. Bundles load lazily and stay loaded. `add_entry` registers an already loaded entry (in-process plugins under test).
- `ClapFactory` creates instances: instantiate, load stored state (state extension required when state is given), validate and clamp node-data parameter values against the params extension (unknown or read-only ids are errors), request the render mode (offline for deterministic, real-time for streaming; a refusal is not an error), read audio-port layout, activate, read latency.
- `ClapProcessor` implements `Processor`: buffers for every audio port, node-data parameter values as CLAP param-value events at frame 0 of the first block, note events as CLAP note-on/off on note port 0 channel 0, tail from the tail extension (unknown without it). Dropping it deactivates and destroys the instance on the main thread.
- All main-thread calls for all instances run on one process-wide CLAP main thread, which also services `request_callback`. Host logging goes to stderr. The host params extension is declared as a no-op; `ClapFactory::params` lists parameters (id, name, module, range, default, flags) from an unactivated instance.
- Plugin editors: `ClapHost::open_editor` opens a plugin's GUI on its own unactivated instance (node settings applied, parameters flushed), embedded in a `winit` host window when the main thread runs one and the plugin supports embedding, else floating. Changes the plugin reports (`mark_dirty`, `request_flush`, a values rescan) are saved `SAVE_DELAY` (300 ms) after the last one and on close, and reported as `EditorEvent::State`; `EditorHandle` closes or focuses an editor. The host declares the `gui`, `state`, and `params` extensions.
- The CLAP main thread runs a `winit` event loop when a display opens (X11 on Linux; Win32 messages pumped), else the headless job loop.
- `aura_clap_fixture` (test-only, `publish = false`) provides deterministic CLAP plugins: `aura.fixture.sine` (polyphonic sine, `Level` parameter id 0, 64-frame latency, 0.1 s release) and `aura.fixture.gain` (gain in its state; a floating fake GUI with `simulate_gui_edit` for editor tests). It builds as a `.clap` bundle (`cdylib`) and loads in-process in tests.

## Non-goals / avoided approaches

- One main thread per instance (breaks plugins that assume a single main thread)
- Mode handling (belongs in `aura_plugin_host`)
- Out-of-process hosting (deferred)

## See also

- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_clap`](../../../../crates/aura_clap), [`../../../../crates/aura_clap_fixture`](../../../../crates/aura_clap_fixture)
