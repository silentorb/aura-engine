# aura_plugin_host — scoping

The translation between Aura's seekable, score-aware model and plugins' forward-only streaming model is the same for every plugin protocol. Keeping it in one protocol-neutral crate means VST3 or LV2 later are backends, not redesigns. See [`../../../scoping/plugin-hosting.md`](../../../scoping/plugin-hosting.md).
