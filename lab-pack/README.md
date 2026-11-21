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
```

Place this directory’s contents next to `imp-lab-tauri` (or set `IMP_LAB_EXTENSIONS` to the `extensions.json` path). Lab runs without the pack (Imp core only). With the pack loaded, Lab gains Aura node catalogs and a **Play** menu (Start / Stop) that drives CPAL playback for Imp graphs and Aura `notation` tips (piano-roll audition from the parked playhead).

## Build the extension libraries

From the aura-engine workspace:

```bash
cargo build -p aura_imp_lab_catalogs -p aura_imp_lab_playback
# Linux: target/debug/libaura_imp_lab_catalogs.so
#         target/debug/libaura_imp_lab_playback.so
# Windows (MSVC): aura_imp_lab_catalogs.dll / aura_imp_lab_playback.dll
```

Copy the artifacts into `lab-pack/extensions/`. The host resolves bare `libraryPath` stems across `.so` / `.dll` / `.dylib` names.

## See also

- Imp Lab extensions: `imp-lab/docs/features/extensions.md`
- Crates: `crates/aura_imp_lab_catalogs`, `crates/aura_imp_lab_playback`
