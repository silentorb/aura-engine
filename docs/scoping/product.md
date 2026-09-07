# Product scoping

Motivation and goals for Aura — procedural music composition and synthesis software.

## Vision

Aura is a library-first framework for procedural music. Specialized programs compose Aura crates to produce audio; notation and synthesis integrate through explicit, documented pipelines rather than hidden coupling.

## Goals

- Express music as procedural, reusable patterns rather than destructive stored sequences
- Model synthesis as pure **Time → Sample** functions via Imp graphs
- Support deterministic offline rendering with optional future-aware scheduling
- Keep integration at the surface; core crates stay focused

Refined requirements live in [`../design/product.md`](../design/product.md), [`../design/notation-synthesis.md`](../design/notation-synthesis.md), and [`../design/imp-execution.md`](../design/imp-execution.md).

## Open questions

- Live vs offline parity when lookahead is unavailable
- Graph-owned duration vs external render parameters
- Buffer sampling optimizations preserving FP semantics
