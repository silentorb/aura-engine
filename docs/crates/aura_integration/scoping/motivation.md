# aura_integration — scoping

Integration belongs at the surface so core crates stay focused. An indirect dependency is a hidden direct dependency — this crate (plus thin CLI/demos) owns load → translate → sample → WAV wiring.

Declarative Imp graphs need a host-specific audio meaning: Aura chooses pure Time → Sample composition rather than tabular `imp_execution` or FunDSP tick lowering.

See [`../../../scoping/background.md`](../../../scoping/background.md).
