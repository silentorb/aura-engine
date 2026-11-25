# aura_dsp — design

## Summary

Pure per-frame DSP functions of time (`sine`, sweeps, noise, oscillators, …), evaluated by node signals for each frame of a block.

## Requirements

- Every public DSP entry must be a pure function of explicit time (and parameters): seekable `f(t)`.
- No hidden mutable phase or filter state in this crate. A history-dependent algorithm (a streaming implementation) keeps its state in its node signal ([node state](../../../design/imp-execution.md#node-state-one-rule-per-mode)).
- Offline and future real-time paths share these functions.

## Non-goals / avoided approaches

- FunDSP-style sequential `tick` / `AudioUnit` state machines in this crate
- File I/O or Imp graph ownership

## See also

- [`../../../design/imp-execution.md`](../../../design/imp-execution.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_dsp`](../../../../crates/aura_dsp)
