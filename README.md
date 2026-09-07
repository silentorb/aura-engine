# Aura Engine

Procedural music composition and synthesis software (Rust).

This repository is the **engine** product (`aura-engine`). Primary development opens the sibling **aura-workbench** host folder, which starts engine and editor containers via Docker Compose.

## Status

Early / pre-alpha. Multi-crate workspace with Imp graph-driven offline WAV rendering.

## Language

Rust (primary).

## Development environment

Do **not** open this repo alone as the Dev Container entrypoint. Use **aura-workbench**:

1. Open `~/dev/aura-workbench` in Cursor.
2. Reopen in Container (Compose starts `engine` + `editor`).
3. Work in `/workspaces/aura-workbench/.mnt/aura-engine` (this tree).

Imp siblings are bind-mounted under the workbench `.mnt/` tree and again under this repo’s `.mnt/` (`imp-rust`, `imp-spec`, `imp-ts`) so Cargo path deps resolve `imp-rust` from `.mnt/imp-rust`.

Expected host layout:

```text
~/dev/
  aura-workbench/
  aura-engine/
  aura-editor/
  imp-rust/
  imp-spec/
  imp-ts/
```

## Quick start

From `/workspaces/aura-workbench/.mnt/aura-engine` in the workbench engine service:

```bash
cargo build
cargo test
cargo clippy
./demos/render-all.sh
```

This renders `demos/demo-01.json` to `output/demo-01.wav` (32-bit float, mono, 44100 Hz). The `output/` directory is gitignored.

### CLI

```bash
cargo run -p aura_cli -- \
  --graph demos/demo-01.json \
  --output output/demo-01.wav \
  --duration 2m \
  --tempo 120 \
  --sample-rate 44100
```

```
aura --graph PATH --output PATH [OPTIONS]

  -g, --graph PATH         Imp graph JSON file
  -o, --output PATH        Output WAV path
  -d, --duration SPEC      Seconds (10, 10s) or measures (2m)
  --tempo BPM              Tempo for measure durations (default: 120)
  -r, --sample-rate HZ     Sample rate (default: 44100)
  -h, --help               Show help
```

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `aura_sample` | DASP sample primitives and timing types |
| `aura_sequencing` | Notation types and musical time |
| `aura_composition` | Procedural composition generators |
| `aura_scheduler` | Offline event scheduling |
| `aura_instrumentation` | Per-note instruments and mix-down |
| `aura_imp` | Imp node libraries and JSON helpers |
| `aura_dsp` | Pure Time → Sample DSP functions |
| `aura_render` | Offline sampling via `Sampler` |
| `aura_io_wav` | WAV file I/O |
| `aura_io_cpal` | CPAL playback stub (deferred) |
| `aura_integration` | Imp graph translation and render glue |
| `aura_cli` | Thin graph-driven CLI |

## Dependency graph

```mermaid
flowchart TD
  subgraph external [External]
    impCore[imp_core_types]
    impReg[imp_registry]
  end

  sample[aura_sample]
  sequencing[aura_sequencing]
  composition[aura_composition]
  scheduler[aura_scheduler]
  instrumentation[aura_instrumentation]
  auraImp[aura_imp]
  dsp[aura_dsp]
  render[aura_render]
  ioWav[aura_io_wav]
  integration[aura_integration]
  cli[aura_cli]

  auraImp --> impCore
  auraImp --> impReg
  composition --> sequencing
  scheduler --> sequencing
  scheduler --> sample
  instrumentation --> scheduler
  instrumentation --> dsp
  instrumentation --> sample
  render --> dsp
  render --> sample
  ioWav --> render
  ioWav --> sample
  integration --> auraImp
  integration --> composition
  integration --> scheduler
  integration --> instrumentation
  integration --> dsp
  integration --> render
  integration --> ioWav
  cli --> integration
  cli --> sample
```

## Documentation

- [`docs/`](docs/) — scoping (motivation) and design (declarative requirements), including per-crate docs
- [`AGENTS.md`](AGENTS.md) — agent workflow and conventions

## License

MIT — see [LICENSE](LICENSE). Copyright (c) 2026 Silent Orb.
