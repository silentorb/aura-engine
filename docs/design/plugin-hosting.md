# Plugin hosting

Aura hosts third-party audio plugins as Imp nodes in both playback modes. Motivation: [`../scoping/plugin-hosting.md`](../scoping/plugin-hosting.md). Related: [`playback-modes.md`](playback-modes.md), [`imp-execution.md`](imp-execution.md).

## Layers

| Crate | Owns |
|---|---|
| [`aura_plugin_host`](../crates/aura_plugin_host/design/overview.md) | Protocol-neutral seam: `Processor`, `ProcessorFactory`, `PluginResolver`; `RenderCache` (deterministic); `PluginStage` (streaming); score → note events; mono adaptation |
| [`aura_clap`](../crates/aura_clap/design/overview.md) | CLAP backend: discovery, loading, the shared CLAP main thread, instance lifecycle, process, state, latency, tail, render mode |
| `aura_imp` | `aura.plugin` node library |
| `aura_integration` | Translating plugin nodes in both modes; `StreamingProgram::open` |

- All mode handling lives in `aura_plugin_host` and is written against `Processor`; no protocol code leaks into it.
- Each protocol is a backend implementing `Processor`, `ProcessorFactory`, and `PluginResolver`. CLAP is the only backend today.
- Hosts pass a `PluginResolver` to translation (`TranslateOptions::plugins`, `StreamingProgram::prepare_with_plugins`). The CLI and the Lab playback extension use `aura_clap::ClapHost`.

## Plugin nodes

| Node type | Inputs | Output |
|---|---|---|
| `plugin_instrument` | `score`, `plugin`, `state` | `audio` |
| `plugin_effect` | `audio`, `plugin`, `state` | `audio` |

- `plugin` is a literal naming the plugin (for CLAP, its plugin id). Empty is a translation error.
- `state` is a literal holding the plugin's saved state, base64-encoded; empty means the plugin's defaults. Aura treats it as opaque. A plugin that rejects it, or has no state support, is a translation error.
- A plugin the resolver cannot find is a translation error naming the node and plugin id. Translating plugin nodes without a resolver is an error, never silence.
- A plugin instrument **consumes a whole score** and does its own polyphony. This is the stated exception to per-note instancing ([Rule 1](notation-synthesis.md#rule-1-per-note-instances)).

## Deterministic mode: render cache

A plugin node's deterministic implementation is a `RenderCache`: a `Sampler` that owns one plugin instance, renders it forward in blocks into a retained buffer, and reads `at(t)` from that buffer.

- Sampling past the rendered range renders more blocks first. Everything rendered is kept for the life of the translation, so out-of-order and repeated sampling return identical values without re-rendering.
- `at(t)` off the frame grid interpolates linearly between frames.
- Note events come from the fully known score at **exact frames** (`NoteEventSource`): note-offs before note-ons at the same frame; unique note ids per note instance, including per loop cycle of a looping score.
- The plugin's reported **latency is removed** from the head of the output, so output frame `k` is the plugin's response to input and events at frame `k`.
- Upstream inputs (an effect's `audio`, the score) stay pure and are sampled at whatever frames each block needs.
- After the last note, with no audio input, rendering stops once the plugin's reported tail has passed and the output has been quiet for 0.25 s (or after 30 s regardless); later samples are silence. An effect with audio input never stops early.
- Plugins are activated in the protocol's **offline** render mode where supported.
- A plugin that fails while processing stops its cache; the rest of that node's output is silence and the failure is reported (`RenderCache::failure`, and on stderr). Errors at creation fail translation.
- Reproducibility is **as strong as the plugins in the graph**. Aura adds no nondeterminism of its own: the same graph, state, and plugin build render bit-identical output.

## Streaming mode: shared instances

Streaming is plugins' native model; Aura supplies the graph split, decided once by `StreamingProgram`:

| Graph shape at the output | Runtime |
|---|---|
| `notation → plugin_instrument → plugin_effect* → output` | One shared `PluginStage` per plugin. Note commands go straight to the instrument stage; effects chain after it. Nothing is per-note. |
| `… per-note nodes → plugin_effect+ → output` | Per-note voices of the effect chain's input mix in a `VoiceMixer`; each effect runs once over the mix. |
| Anything else with plugin nodes | **Per-voice fallback**: each voice instance gets its own plugin instances. An instrument plays the voice's live note (note-on at onset, note-off at release). Correct, but costly. |

- `StreamingProgram::open` returns a `StreamingControl` (note-on / note-off / all-off from a control thread) and the `BlockSource` to play. `StreamingProgram::instantiate` is always the per-voice form.
- Translating for streaming never fails because of plugins; the fallback always applies.
- A `PluginStage` drains commands at the start of each block; they take effect at the block's first frame. It never allocates on the audio thread.
- Plugins in streaming are activated in **real-time** render mode.
- In the per-voice fallback a release takes effect at the next block the plugin renders (at most one block late).

## Channels

Aura is mono end to end. A mono input is copied to every channel of the plugin's main input port; the output is the average of the main output port's channels. Other ports receive silence and are ignored.

## CLAP backend

- `ClapHost` searches its extra paths, then `CLAP_PATH`, then the platform's standard locations (Linux `~/.clap`, `/usr/lib/clap`; Windows `%COMMONPROGRAMFILES%\CLAP`, `%LOCALAPPDATA%\Programs\Common\CLAP`; macOS `~/Library/Audio/Plug-Ins/CLAP`, `/Library/Audio/Plug-Ins/CLAP`). Bundles load lazily on first discovery or resolve and stay loaded for the host's life. Bundles that fail to load are reported, not fatal.
- Every instance's main-thread half runs on **one process-wide CLAP main thread** (frameworks such as JUCE assume one). Only the audio processor leaves it; dropping a processor deactivates and destroys its instance there. That thread also services plugins' `request_callback`.
- Plugins run **in-process**.

## Non-goals / avoided approaches

- One plugin instance per note in streaming, except as the documented per-voice fallback
- Catching a plugin up on the audio thread during real-time transport playback (phase 3 adds render-ahead; until then transport playback of plugin graphs may glitch at start)
- Depending on VST3 → CLAP adapters for other protocols; they are backends behind `Processor`
- Protocol-specific code in `aura_plugin_host`, `aura_imp`, or the translator
- Interpreting plugin state blobs

## Open questions

- Per-plugin generated node types with one port per parameter; automation and per-note parameter mapping (phase 2)
- Render-ahead and preparation for transport playback (phase 3)
- Plugin GUIs and capturing state into the node on edit (phase 4)
- Out-of-process sandboxing
- Multichannel Aura audio
- Bounding cache memory for long renders (currently about 10.6 MB per plugin node per minute at 44.1 kHz)
