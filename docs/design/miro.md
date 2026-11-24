# Miro (Aura host)

Declarative requirements for Aura’s use of Mir/Miro. Motivation: multi-representation data (text authoring, query projection, pull working set) without Aura owning the sync fabric.

## Requirements

- Mir fabric and the generic Miro text→database→pull profile **must** live in **`imp_mir`** (hosted in tome-rust-server). Aura **must not** reimplement store registry, sync signals, or `mir.sync.store` wire-up.
- Aura **must** treat on-disk Aura JSON (`imp-graph`, `notation`, `imp-graph-library`, …) as the Miro text source of truth for this product.
- Aura **must** use SQLite as the Miro database projection (via `imp_mir`’s generic SQLite store).
- Aura host helpers **may** map Aura JSON `type` into opaque Mir record `kind` and pull Imp `Graph` values through `aura_imp` JSON helpers.
- Direct `load_graph` / filesystem open paths **must** keep working; Miro is an additive parallel session path.
- Mir `pull_record` takes an opaque Mir record id — never treat a NodeId as a filesystem path.
- Aura **must not** resolve NodeIds a graph references but does not hold. That is an Imp and Tome concern: translation reads a graph source (the document layered with Tome's `tome_imp` node provider over a Tome project), and Aura only contributes node types (e.g. the notation node type, whose id is the Notation type node).
- Opaque Mir record ids **may** be encoded by the text store as root-relative paths; that encoding is host/store policy, not a Mir-normative or product API.

## Non-goals / avoided approaches

- An Aura host NodeId index or `NodeResolver` over a content root (or any other Aura-side node resolution, including synthetic definitions such as `#aura/notation`): node resolution belongs to Imp graph sources and Tome
- An `aura_mir` fabric crate that duplicates `imp_mir`
- Encoding Aura path layouts or SQL schemas as Mir-normative requirements
- Promoting path-shaped Mir blob keys as the editor/product open identity
- `mir.sync.gitWatch` (deferred)
- Replacing editor document open/save with SQLite as the only path
- Temporary File-menu / path-picker Miro open UX that would be removed when real discovery or node-ref UI lands

## See also

- [Mir specs](https://github.com/silentorb/mir) (`docs/features/miro.md`, `imp-sync-wiring.md`)
- `imp_mir` crate README in [tome-rust-server](https://github.com/silentorb/tome-rust-server) (`crates/imp_mir`)
- [`architecture.md`](architecture.md)
- Code: [`../../crates/aura_integration/src/host.rs`](../../crates/aura_integration/src/host.rs)
