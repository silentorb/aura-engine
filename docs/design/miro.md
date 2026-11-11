# Miro (Aura host)

Declarative requirements for Aura’s use of Mir/Miro. Motivation: multi-representation data (text authoring, query projection, pull working set) without Aura owning the sync fabric.

## Requirements

- Mir fabric and the generic Miro text→database→pull profile **must** live in **`imp_mir`** (imp-rs). Aura **must not** reimplement store registry, sync signals, or `mir.sync.store` wire-up.
- Aura **must** treat on-disk Aura JSON (`imp-graph`, `notation`, `imp-graph-library`, …) as the Miro text source of truth for this product.
- Aura **must** use SQLite as the Miro database projection (via `imp_mir`’s generic SQLite store).
- Aura host helpers **may** map Aura JSON `type` into opaque Mir record `kind` and pull Imp `Graph` values through `aura_imp` JSON helpers.
- Direct `load_graph` / filesystem open paths **must** keep working; Miro is an additive session path.
- App memory pull into the editor `ModelGraph` is deferred; engine/CLI pull is sufficient for this slice.

## Non-goals / avoided approaches

- An `aura_mir` fabric crate that duplicates `imp_mir`
- Encoding Aura path layouts or SQL schemas as Mir-normative requirements
- `mir.sync.gitWatch` (deferred)
- Replacing editor document open/save with SQLite in this slice

## See also

- [Mir specs](https://github.com/silentorb/mir) (`docs/features/miro.md`, `imp-sync-wiring.md`)
- `imp_mir` crate README in imp-rs
- [`architecture.md`](architecture.md)
- Code: [`../../crates/aura_integration/src/miro.rs`](../../crates/aura_integration/src/miro.rs)
