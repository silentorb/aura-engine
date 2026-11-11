# aura_integration — scoping

Integration belongs at the surface so core crates stay focused. An indirect dependency is a hidden direct dependency — this crate (plus thin CLI/demos) owns load → translate → sample → WAV wiring.

Declarative Imp graphs need a host-specific audio meaning: Aura chooses pure Time → Sample composition rather than tabular `imp_execution` or FunDSP tick lowering.

Miro text→SQLite sessions are also surface glue: call `imp_mir`, map Aura JSON kinds, pull graphs — the sync fabric stays out of Aura domain crates. See [`../../../scoping/background.md`](../../../scoping/background.md) and [`../../../design/miro.md`](../../../design/miro.md).
