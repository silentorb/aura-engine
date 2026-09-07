# aura_dsp — design

## Summary

Pure Time → Sample DSP functions (`sine`, sweeps, noise, oscillators, …).

## Requirements

- Every public DSP entry must be a pure function of explicit time (and parameters): seekable `f(t)`.
- No hidden mutable phase or filter state between `Sampler::at(t)` calls.
- Offline and future real-time paths share these functions.

## Non-goals / avoided approaches

- FunDSP-style sequential `tick` / `AudioUnit` state machines
- File I/O or Imp graph ownership

## See also

- [`../../../design/imp-execution.md`](../../../design/imp-execution.md)
- Scoping: [`../scoping/motivation.md`](../scoping/motivation.md)
- Code: [`../../../../crates/aura_dsp`](../../../../crates/aura_dsp)
