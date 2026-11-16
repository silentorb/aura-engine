# Miro (Aura host)

Declarative requirements for Aura’s use of Mir/Miro. Motivation: multi-representation data (text authoring, query projection, pull working set) without Aura owning the sync fabric.

## Requirements

- Mir fabric and the generic Miro text→database→pull profile **must** live in **`imp_mir`** (imp-rs). Aura **must not** reimplement store registry, sync signals, or `mir.sync.store` wire-up.
- Aura **must** treat on-disk Aura JSON (`imp-graph`, `notation`, `imp-graph-library`, …) as the Miro text source of truth for this product.
- Aura **must** use SQLite as the Miro database projection (via `imp_mir`’s generic SQLite store).
- Aura host helpers **may** map Aura JSON `type` into opaque Mir record `kind` and pull Imp `Graph` values through `aura_imp` JSON helpers.
- Direct `load_graph` / filesystem open paths **must** keep working; Miro is an additive parallel session path.
- Product open / resolve keys **must** be Imp **NodeId**s. Aura **must** maintain a host index `NodeId → { mir_record_id, kind }` over the content root (`imp-graph` local node ids; `notation` tip `id`). Mir `pull_record` takes the opaque Mir record id from that index — never treat a NodeId as a filesystem path.
- Opaque Mir record ids **may** be encoded by the text store as root-relative paths; that encoding is host/store policy, not a Mir-normative or product API.
- The editor **must** expose a durable Miro pull workflow into app memory (`ModelGraph`): open a `MiroSession` for a content root, resolve a NodeId through the host index, pull the Mir blob, and materialize it through the stacked document-type pipeline. Product File → Open may remain filesystem-only until a later slice attaches UI to that workflow.
- Editor app memory for Imp graphs **must** treat `ModelGraph.graph.nodes` as the Imp **local** instance map for that tab. Pulled JSON **must** preserve edge topology even when endpoints lack a local node entry (missing cards / typecheck errors). Miro-backed resolution of those missing ids is later host policy, not a Mir observe sink.
- Notation tips used with Miro **must** carry a stable tip-level `id` (NodeId) so they can be indexed without colliding on a fixed placeholder.

## Non-goals / avoided approaches

- An `aura_mir` fabric crate that duplicates `imp_mir`
- Encoding Aura path layouts or SQL schemas as Mir-normative requirements
- Promoting path-shaped Mir blob keys as the editor/product open identity
- `mir.sync.gitWatch` (deferred)
- Replacing editor document open/save with SQLite as the only path
- Temporary File-menu / path-picker Miro open UX that would be removed when real discovery or node-ref UI lands

## See also

- [Mir specs](https://github.com/silentorb/mir) (`docs/features/miro.md`, `imp-sync-wiring.md`)
- `imp_mir` crate README in imp-rs
- [`architecture.md`](architecture.md)
- Code: [`../../crates/aura_integration/src/miro.rs`](../../crates/aura_integration/src/miro.rs)
