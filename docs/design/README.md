# Design

Declarative requirements that define Aura. Design *is* the requirements surface — refined from scoping for day-to-day agent and human use.

See [Silent Orb: Scoping and Design](https://silentorb.com/business/product/scoping-and-design/) and [`../README.md`](../README.md).

## What belongs here

- Declarative product and architecture requirements
- Module boundaries, data flow, and behavioral rules
- Non-goals and avoided approaches (brief, timeless)
- Links to [`../scoping/`](../scoping/) for motivation origins

## What does not belong here

- Raw brainstorming and unrefined notes (see scoping)
- Dated append-only decision logs
- Line-by-line implementation detail

## Style

- State what must be true **now**, in present tense.
- Prefer edit-in-place over historical supersession records.
- Mark undecided areas as TBD or open questions.
- Link crate-local detail under [`../crates/`](../crates/).

## Documents

| Document | Description |
|----------|-------------|
| [`product.md`](product.md) | Product-level declarative requirements |
| [`architecture.md`](architecture.md) | System overview, crate layout, data flow |
| [`imp-execution.md`](imp-execution.md) | Imp graph Time → Sample execution model |
| [`notation-synthesis.md`](notation-synthesis.md) | Notation → audio pipeline and integration rules |
| [`../rust-style.md`](../rust-style.md) | Rust crate naming and layout conventions |
