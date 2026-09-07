# Crate docs

Per-crate agent documentation. Each crate has:

- **`scoping/`** — crate-local motivation and context (optional until needed)
- **`design/`** — crate-local declarative requirements

Cross-cutting product motivation and architecture live in [`../scoping/`](../scoping/) and [`../design/`](../design/). Prefer crate docs for ownership-local detail; prefer root docs for shared pipelines and product-wide rules.

## Registry

| Crate | Design | Scoping |
|-------|--------|---------|
| `aura_sample` | [design/overview.md](aura_sample/design/overview.md) | [scoping/](aura_sample/scoping/) |
| `aura_sequencing` | [design/overview.md](aura_sequencing/design/overview.md) | [scoping/](aura_sequencing/scoping/) |
| `aura_composition` | [design/overview.md](aura_composition/design/overview.md) | [scoping/](aura_composition/scoping/) |
| `aura_scheduler` | [design/overview.md](aura_scheduler/design/overview.md) | [scoping/](aura_scheduler/scoping/) |
| `aura_instrumentation` | [design/overview.md](aura_instrumentation/design/overview.md) | [scoping/](aura_instrumentation/scoping/) |
| `aura_imp` | [design/overview.md](aura_imp/design/overview.md) | [scoping/motivation.md](aura_imp/scoping/motivation.md) |
| `aura_dsp` | [design/overview.md](aura_dsp/design/overview.md) | [scoping/motivation.md](aura_dsp/scoping/motivation.md) |
| `aura_render` | [design/overview.md](aura_render/design/overview.md) | [scoping/](aura_render/scoping/) |
| `aura_io_wav` | [design/overview.md](aura_io_wav/design/overview.md) | [scoping/motivation.md](aura_io_wav/scoping/motivation.md) |
| `aura_io_cpal` | [design/overview.md](aura_io_cpal/design/overview.md) | [scoping/motivation.md](aura_io_cpal/scoping/motivation.md) |
| `aura_integration` | [design/overview.md](aura_integration/design/overview.md) | [scoping/motivation.md](aura_integration/scoping/motivation.md) |
| `aura_cli` | [design/overview.md](aura_cli/design/overview.md) | [scoping/](aura_cli/scoping/) |

## Authoring

When adding a crate: create `docs/crates/<crate>/design/overview.md`, add a registry row here and in root [`AGENTS.md`](../../AGENTS.md), and add `scoping/` notes when motivation is crate-specific.
