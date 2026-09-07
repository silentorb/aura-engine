# aura_dsp — scoping

Seekable pure `f(t)` sampling is a product goal. Sequential tick DSP can be offline-deterministic but cannot answer `at(t)` without simulating prior samples. That conflict is why Aura keeps DSP as closed-form (or otherwise stateless-in-t) functions in this crate.

See [`../../../scoping/background.md`](../../../scoping/background.md).
