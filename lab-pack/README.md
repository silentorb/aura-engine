# Aura pack: engine libraries and data

aura-engine's part of the **Aura pack**, the set of extensions that turns [Imp Lab](https://github.com/silentorb/imp-lab) into Aura's editor. The pack **configuration** (`extensions.json`) and Aura's editor UI are owned by [aura-editor](https://github.com/silentorb/aura-editor) (`pack/extensions.json`, `docs/design/pack.md`). This directory holds what the engine supplies:

```
lab-pack/
  extensions/          # local build output (git-ignored artifacts):
                       #   libaura_imp_lab_catalogs.so / aura_imp_lab_catalogs.dll
                       #   libaura_imp_lab_playback.so / aura_imp_lab_playback.dll
  catalogs/
    instruments.json   # Imp file of instrument definitions (audition voice, metronome click, demos)
  demo-project/        # sample Imp Lab project: the Aura demos as records
    imp-project.json   #   File → Open Project… and pick this file
    content/           #   one Tome data store (flat files)
```

aura-workbench assembles the full pack (aura-editor's configuration and UI library, these libraries and data) for a local run (`scripts/assemble-aura-pack.sh`) and for the Windows export beside `imp-lab.exe`.

## What the libraries provide

- **Catalogs** (`aura_imp_lab_catalogs`): Aura node catalogs for Imp Lab's add-node picker, including the `aura.plugin` node library (`plugin_instrument`, `plugin_effect`).
- **Playback** (`aura_imp_lab_playback`, service `playback`): deterministic CPAL playback for Imp graphs and Aura `notation` records (from the parked playhead), driven by the `play.*` commands. The service also accepts `event` messages for live note audition in streaming mode (`note_on` / `note_off` / `all_notes_off`), voiced by the `sine_note_voice` instrument in `catalogs/instruments.json`; see [`../docs/design/playback-modes.md`](../docs/design/playback-modes.md).
- **CLAP plugins** ([`../docs/design/plugin-hosting.md`](../docs/design/plugin-hosting.md)): playback resolves plugin ids from `CLAP_PATH` and the platform's standard CLAP locations (on Windows, `%COMMONPROGRAMFILES%\CLAP`). The service publishes installed plugins on `aura.plugins.instruments` / `aura.plugins.effects`, answers `plugin.open_gui` node actions, and publishes GUI edits of a node's `state` on `aura.plugins.edits`.
- **MIDI input** ([`../docs/design/live-recording.md`](../docs/design/live-recording.md)): settings `aura.midi.input` (options from the `aura.midi.ports` topic) and `aura.midi.channels`, status on `aura.midi.status`. The service answers the live note capture requests (`capture_info`, `record_start`, `record_stop`) and publishes the take on `aura.capture.take`. The recording metronome is the `metronome_click` instrument in `catalogs/instruments.json`.

The configuration that exposes these (commands, menus, settings, node UI, bindings) is in aura-editor; its tests check it against these ids.

## Build the extension libraries

From the aura-engine workspace:

```bash
cargo build -p aura_imp_lab_catalogs -p aura_imp_lab_playback
# Linux: target/debug/libaura_imp_lab_catalogs.so
#         target/debug/libaura_imp_lab_playback.so
# Windows (MSVC): aura_imp_lab_catalogs.dll / aura_imp_lab_playback.dll
```

The host resolves bare `libraryPath` stems across `.so` / `.dll` / `.dylib` names.

## Demo project

`demo-project/` is generated from `../demos/*.json` by Imp Lab's seed tool, with the pack configuration so notation documents become Notations records. Notations keep their ids, so `demo-02` still references `demo-02-arpeggio`. Regenerate it after changing a demo (delete the folder first):

```bash
# from imp-lab/apps/tauri/src-tauri, with aura-engine at $E and aura-editor at $A
cargo run --example seed_project -- --extensions $A/pack/extensions.json \
  $E/lab-pack/demo-project "Aura demos" \
  $E/demos/demo-01.json $E/demos/demo-02.json $E/demos/demo-02-arpeggio.json $E/demos/sine.json
```

Exports copy it to `demo-project/` beside `imp-lab.exe`, but never over an existing copy.

In Imp Lab, Play hands the extension the open project's config. Playback then reads the records a graph references (for example `demo-02`'s arpeggio, by node id) from the project's data stores, and builds instruments from `catalogs/instruments.json`.

## See also

- Pack configuration and editor UI: aura-editor `pack/extensions.json`, `docs/design/pack.md`, `docs/design/piano-roll.md`
- Imp Lab extensions: imp-lab `docs/features/extensions.md`
- Crates: `crates/aura_imp_lab_catalogs`, `crates/aura_imp_lab_playback`
