# aura_dsp — scoping

Seekable deterministic rendering is a product goal. Sequential tick DSP can be offline-deterministic but cannot render an arbitrary range without simulating prior frames. That conflict is why Aura keeps DSP as closed-form (or otherwise stateless-in-t) functions in this crate; history-carrying algorithms belong to streaming implementations ([block processing](../../../scoping/block-processing.md)).

See [`../../../scoping/background.md`](../../../scoping/background.md).
