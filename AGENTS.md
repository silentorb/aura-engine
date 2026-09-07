# Aura Engine — Agent Instructions

## Project

Aura Engine (`aura-engine`) is procedural music composition and synthesis software, written primarily in Rust. This repository is the **engine product**, not the Dev Container entrypoint.

Primary workspace: sibling **aura-workbench** (Compose + Dev Container). Prefer opening workbench; work in `/workspaces/aura-workbench/.mnt/aura-engine` when attached to the engine service.

## Source of truth

[`docs/README.md`](docs/README.md) is the entry point for all project documentation. **Design** docs under `docs/` define what the software must be (declarative requirements); code must conform to them. **Scoping** holds motivation and context. When docs and code disagree, update code or explicitly revise the doc — never silently drift.

## Scoping vs design

Follow [Silent Orb: Scoping and Design](https://silentorb.com/business/product/scoping-and-design/):

- **Scoping** (`docs/scoping/`, `docs/crates/<crate>/scoping/`) — gather motivation, goals, background, inspiration, context (write-friendly).
- **Design** (`docs/design/`, `docs/crates/<crate>/design/`) — refine into declarative requirements that define the product (read-friendly). Design may link to scoping for motivation origins.

Prefer gather then refine. When phases overlap, switch modes consciously. Do not keep an append-only decision log; edit living design in place.

## Capturing durable intent

Treat user instructions that define product behavior, architecture, or conventions as durable. Before or alongside code changes:

1. Capture **why** (goals, intent, background) in the appropriate **scoping** doc.
2. Capture **what must be true** (requirements, non-goals, avoided approaches) in the appropriate **design** doc.
3. Leave one-off task instructions as execution context only.

Plans that change durable intent must include an explicit scoping and/or design update step.

When reversing a direction: add a brief avoided-approach requirement in design; optionally note background in scoping; remove obsolete design prescriptions.

## Working conventions

- Crate names use **snake_case** (`aura_scheduler`); follow [`docs/rust-style.md`](docs/rust-style.md).
- Repos and projects use **kebab-case** (`aura-engine`).

## Commands

```bash
cargo build
cargo test
cargo clippy
```

These apply once a Rust crate scaffold exists.

## Demo artifacts

The listenable output in [`output/`](output/) (`demo-01.wav`) is the primary artifact for verifying demos by ear. They are gitignored but must stay current in the workspace.

After any change to demo graphs (`demos/*.json`), [`demos/render-all.sh`](demos/render-all.sh), or code that affects how those graphs render (composition, integration, imp nodes, CLI, etc.), regenerate them:

```bash
demos/render-all.sh
```

Do not rely on unit tests alone for demo audio — tests assert frame counts and determinism, not that `output/` was refreshed.

## Workspace

Aura development mounts multiple repositories via **aura-workbench**. Each repo may have its own `AGENTS.md` and `docs/`. Prefer the nearest `AGENTS.md` and that repo's `docs/` when working in a given tree. This file governs **aura-engine** only.

Orchestration (Compose, Dev Container, multi-repo mounts) lives in **aura-workbench**, not here.

## Where to look

- [`docs/scoping/`](docs/scoping/) — product motivation, goals, background
- [`docs/design/`](docs/design/) — cross-cutting declarative design
- [`docs/crates/`](docs/crates/) — per-crate scoping and design
- [`docs/rust-style.md`](docs/rust-style.md) — Rust crate naming and layout

### Crate docs

| Crate | Design |
|-------|--------|
| `aura_sample` | [`docs/crates/aura_sample/design/overview.md`](docs/crates/aura_sample/design/overview.md) |
| `aura_sequencing` | [`docs/crates/aura_sequencing/design/overview.md`](docs/crates/aura_sequencing/design/overview.md) |
| `aura_composition` | [`docs/crates/aura_composition/design/overview.md`](docs/crates/aura_composition/design/overview.md) |
| `aura_scheduler` | [`docs/crates/aura_scheduler/design/overview.md`](docs/crates/aura_scheduler/design/overview.md) |
| `aura_instrumentation` | [`docs/crates/aura_instrumentation/design/overview.md`](docs/crates/aura_instrumentation/design/overview.md) |
| `aura_imp` | [`docs/crates/aura_imp/design/overview.md`](docs/crates/aura_imp/design/overview.md) |
| `aura_dsp` | [`docs/crates/aura_dsp/design/overview.md`](docs/crates/aura_dsp/design/overview.md) |
| `aura_render` | [`docs/crates/aura_render/design/overview.md`](docs/crates/aura_render/design/overview.md) |
| `aura_io_wav` | [`docs/crates/aura_io_wav/design/overview.md`](docs/crates/aura_io_wav/design/overview.md) |
| `aura_io_cpal` | [`docs/crates/aura_io_cpal/design/overview.md`](docs/crates/aura_io_cpal/design/overview.md) |
| `aura_integration` | [`docs/crates/aura_integration/design/overview.md`](docs/crates/aura_integration/design/overview.md) |
| `aura_cli` | [`docs/crates/aura_cli/design/overview.md`](docs/crates/aura_cli/design/overview.md) |
