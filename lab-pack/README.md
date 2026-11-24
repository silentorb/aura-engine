# Imp Lab pack (Aura audio catalogs + playback)

Optional sidecar pack for [Imp Lab Tauri](https://github.com/silentorb/imp-lab): Tome-shaped `extensions.json` plus Aura catalog and playback cdylibs.

## Layout

```
lab-pack/
  extensions.json
  extensions/          # platform cdylibs:
                       #   libaura_imp_lab_catalogs.so / aura_imp_lab_catalogs.dll
                       #   libaura_imp_lab_playback.so / aura_imp_lab_playback.dll
  catalogs/
    instruments.json   # optional demo graph library
  demo-project/        # sample Imp Lab project: the Aura demos as records
    imp-project.json   #   File → Open Project… and pick this file
    content/           #   one Tome data store (flat files)
```

Place this directory’s contents next to `imp-lab-tauri` (or set `IMP_LAB_EXTENSIONS` to the `extensions.json` path). Lab runs without the pack (Imp core only). With the pack loaded, Lab gains Aura node catalogs and a **Play** menu (Start / Stop) that drives deterministic CPAL playback for Imp graphs and Aura `notation` tips (from the parked playhead). The playback service also accepts `event` messages for live note audition in streaming mode (`note_on` / `note_off` / `all_notes_off`), voiced by the `sine_note_voice` instrument in `catalogs/instruments.json`; see [`../docs/design/playback-modes.md`](../docs/design/playback-modes.md).

The pack also brings **CLAP plugins** ([`../docs/design/plugin-hosting.md`](../docs/design/plugin-hosting.md)): the catalogs include the `aura.plugin` node library (`plugin_instrument`, `plugin_effect`), and playback resolves plugin ids from `CLAP_PATH` and the platform's standard CLAP locations (on Windows, `%COMMONPROGRAMFILES%\CLAP`). Choosing plugins in the UI, opening plugin GUIs, and capturing their state are later phases; for now a plugin node's `plugin` id and base64 `state` are set as literals.

The pack also brings **MIDI input** ([`../docs/design/live-recording.md`](../docs/design/live-recording.md)). `extensions.json` registers a **MIDI** settings category with two settings, **Input device** (`aura.midi.input`, options from the `aura.midi.ports` topic) and **Channels** (`aura.midi.channels`), and a `status-item` showing the `aura.midi.status` topic. The playback service answers the live note capture requests (`capture_info`, `record_start`, `record_stop`) and publishes the take on `aura.capture.take`. The recording metronome is the `metronome_click` instrument in `catalogs/instruments.json`.

## Build the extension libraries

From the aura-engine workspace:

```bash
cargo build -p aura_imp_lab_catalogs -p aura_imp_lab_playback
# Linux: target/debug/libaura_imp_lab_catalogs.so
#         target/debug/libaura_imp_lab_playback.so
# Windows (MSVC): aura_imp_lab_catalogs.dll / aura_imp_lab_playback.dll
```

Copy the artifacts into `lab-pack/extensions/`. The host resolves bare `libraryPath` stems across `.so` / `.dll` / `.dylib` names.

## Demo project

`demo-project/` is generated from `../demos/*.json` by Imp Lab's seed tool. Notations keep their ids, so `demo-02` still references `demo-02-arpeggio`. Regenerate it after changing a demo (delete the folder first):

```bash
# from imp-lab/apps/tauri/src-tauri, with aura-engine at $E
cargo run --example seed_project -- $E/lab-pack/demo-project "Aura demos" \
  $E/demos/demo-01.json $E/demos/demo-02.json $E/demos/demo-02-arpeggio.json $E/demos/sine.json
```

The Windows export copies it to `demo-project/` beside `imp-lab.exe`, but never over an existing copy.

In Imp Lab, Play hands the extension the open project's config. Playback then reads the records a graph references (for example `demo-02`'s arpeggio, by node id) from the project's data stores, and builds instruments from `catalogs/instruments.json`.

## See also

- Imp Lab extensions: `imp-lab/docs/features/extensions.md`
- Crates: `crates/aura_imp_lab_catalogs`, `crates/aura_imp_lab_playback`
