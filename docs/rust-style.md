# Rust style guide

Living style for **Aura** crates. Naming aligns with sibling [imp-rust](../.mnt/imp-rust/docs/rust-style.md).

## Naming and layout

- Crates live under `crates/` with **snake_case** names (`aura_sample`, `aura_scheduler`, …).
- Same for `[package] name`, workspace members, dependency keys, and Cargo `-p` package selectors.
- Do not use kebab-case (`aura-scheduler`) for crate directories or package names.
- The CLI package is `aura_cli`; the installed binary may remain `aura` via `[[bin]]`.

## Errors

Error handling aligns with Imp’s [Errors](../.mnt/imp-rust/docs/rust-style.md#errors) section. In short:

- Prefer `Result` / explicit status for **expected** failure; panic only for programmer invariants or documented fail-fast aborts.
- Prefer precise, explicit error flows over broad catch-all handling.
- Propagate errors with `?` unless the current layer can meaningfully recover.
- Handle errors locally only with an intentional recovery or translation strategy.
- Preserve error causes and useful context.
- Use typed errors when callers need to distinguish failure cases.
- Avoid silently ignored errors, arbitrary fallbacks, and unjustified `unwrap()`/`expect()`.
