# Rust style guide

Living style for **Aura** crates. Naming aligns with sibling [imp-rust](../.mnt/imp-rust/docs/rust-style.md).

## Naming and layout

- Crates live under `crates/` with **snake_case** names (`aura_sample`, `aura_scheduler`, …).
- Same for `[package] name`, workspace members, dependency keys, and Cargo `-p` package selectors.
- Do not use kebab-case (`aura-scheduler`) for crate directories or package names.
- The CLI package is `aura_cli`; the installed binary may remain `aura` via `[[bin]]`.
