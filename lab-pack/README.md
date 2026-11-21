# Imp Lab pack (Aura audio catalogs)

Optional sidecar pack for [Imp Lab Tauri](https://github.com/silentorb/imp-lab): Tome-shaped `extensions.json` plus the `aura_imp_lab_catalogs` cdylib.

## Layout

```
lab-pack/
  extensions.json
  extensions/          # platform cdylib (libaura_imp_lab_catalogs.so / aura_imp_lab_catalogs.dll)
  catalogs/
    instruments.json   # optional demo graph library
```

Place this directory’s contents next to `imp-lab-tauri` (or set `IMP_LAB_EXTENSIONS` to the `extensions.json` path). Lab runs without the pack (Imp core only).

## Build the extension library

From the aura-engine workspace:

```bash
cargo build -p aura_imp_lab_catalogs
# Linux: target/debug/libaura_imp_lab_catalogs.so
# Windows (MSVC): aura_imp_lab_catalogs.dll
```

Copy the artifact into `lab-pack/extensions/`. The host resolves bare `libraryPath` stems across `.so` / `.dll` / `.dylib` names.

## See also

- Imp Lab extensions: `imp-lab/docs/features/extensions.md`
- Crate: `crates/aura_imp_lab_catalogs`
