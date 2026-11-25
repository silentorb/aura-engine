# Plugin hosting

Aura hosts third-party audio plugins as Imp nodes in both playback modes. Motivation: [`../scoping/plugin-hosting.md`](../scoping/plugin-hosting.md). Related: [`playback-modes.md`](playback-modes.md), [`imp-execution.md`](imp-execution.md).

## Layers

| Crate | Owns |
|---|---|
| [`aura_plugin_host`](../crates/aura_plugin_host/design/overview.md) | Protocol-neutral seam: `Processor`, `ProcessorFactory`, `PluginResolver`; `RenderCache` (deterministic); `PluginStage` (streaming); score → note events; layout adaptation (`LayoutStage`) |
| [`aura_clap`](../crates/aura_clap/design/overview.md) | CLAP backend: discovery, loading, the shared CLAP main thread, instance lifecycle, process, state, latency, tail, render mode |
| `aura_imp` | `aura.plugin` node library |
| `aura_integration` | Translating plugin nodes in both modes; `StreamingProgram::open` |

- All mode handling lives in `aura_plugin_host` and is written against `Processor`; no protocol code leaks into it.
- Each protocol is a backend implementing `Processor`, `ProcessorFactory`, and `PluginResolver`. CLAP is the only backend today.
- Hosts pass a `PluginResolver` to translation (`TranslateOptions::plugins`, `StreamingProgram::prepare_with_plugins`). The CLI and the Lab playback extension use `aura_clap::ClapHost`.

## Plugin nodes

| Node type | Inputs | Output |
|---|---|---|
| `plugin_instrument` | `score`, `plugin`, `state`, `params` | `audio` |
| `plugin_effect` | `audio`, `plugin`, `state`, `params` | `audio` |

Plugin nodes have **no plugin-specific ports**. A plugin is configured entirely by embedded node data (`plugin`, `state`, `params`); its only wired inputs are the generic `score` or `audio`.

- `plugin` is a literal naming the plugin (for CLAP, its plugin id). Empty is a translation error.
- `state` is a literal holding the plugin's saved state, base64-encoded; empty means the plugin's defaults. Aura treats it as opaque. A plugin that rejects it, or has no state support, is a translation error.
- `params` is a literal holding parameter values: a JSON object string mapping the plugin's stable parameter id (decimal) to a **plain value** in the plugin's own units, e.g. `{"0": 0.4}`. Empty means none. Values are applied after `state`, overriding it, and reach the plugin as parameter-value events at frame 0 of its first block. Values are clamped to the parameter's range. Malformed `params`, an unknown parameter id, or a read-only parameter is a translation error naming the node.
- Parameter ids come from the plugin (`aura plugin-params <plugin-id>` lists id, name, module, range, default, and flags; `ProcessorFactory::params` is the API).
- A plugin the resolver cannot find is a translation error naming the node and plugin id. Translating plugin nodes without a resolver is an error, never silence.
- A plugin instrument **consumes a whole score** and does its own polyphony. This is the stated exception to per-note instancing ([Rule 1](notation-synthesis.md#rule-1-per-note-instances)).

## Deterministic mode: render cache

A plugin node's deterministic implementation is a `RenderCache`: a block signal ([execution model](imp-execution.md)) that owns one plugin instance, renders it forward from frame 0 on its own block grid (`BLOCK_FRAMES`), keeps what it rendered, and answers any requested range from that.

- A plugin's state is essential and cannot be snapshotted, so a plugin is the one deterministic node that renders from frame 0 ([node state](imp-execution.md#node-state-one-rule-per-mode)). Retaining its output is what lets it meet the deterministic contract: rendering a range past what exists renders more blocks first; everything rendered is kept for the life of the translation, so out-of-order and repeated ranges return identical frames without re-rendering.
- Output is frame-exact: no interpolation.
- Note events come from the fully known score at **exact frames** (`NoteEventSource`): note-offs before note-ons at the same frame; unique note ids per note instance, including per loop cycle of a looping score.
- The plugin's reported **latency is removed** from the head of the output, so output frame `k` is the plugin's response to input and events at frame `k`.
- Upstream inputs (an effect's `audio`, the score) are deterministic, so the plugin renders its audio input over each of its own blocks as it needs it.
- After the last note, with no audio input, rendering stops once the plugin's reported tail has passed and the output has been quiet for 0.25 s (or after 30 s regardless); later frames are silence. An effect with audio input never stops early.
- Plugins are activated in the protocol's **offline** render mode where supported.
- A plugin that fails while processing stops its cache; the rest of that node's output is silence and the failure is reported (`RenderCache::failure`, and on stderr). Errors at creation fail translation.
- Reproducibility is **as strong as the plugins in the graph**. Aura adds no nondeterminism of its own: the same graph, state, and plugin build render bit-identical output.
- A cache holds at most about 3.4 hours (at 44.1 kHz); past that it reads as silence.

## Transport playback: render-ahead

Transport playback (`PlaybackEngine`) plays the deterministic graph in real time without rendering on the audio thread:

- One render thread per playback owns the translated graph and renders it into a lock-free ring buffer, `PREPARE_AHEAD_SECS` (2 s) ahead of the playhead. The audio callback only copies frames out of the ring.
- `PlaybackEngine::start_at` fills the ring on the calling thread before opening the stream. A late start in a long song with plugins waits while they render from frame 0 up to the start.
- If the ring runs dry, the callback plays silence and counts the missing frames (`PlaybackEngine::underruns`); the playhead reports what was heard.
- Output is identical to offline rendering: the same graph renders the same frames. Plugins render in the offline render mode in transport too, so a plugin slower than real time underruns instead of changing its sound.

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

## Imp Lab: choosing plugins and editing them

The Lab pack gives plugin nodes UI through Imp Lab's generic node UI components (Lab knows nothing of plugins):

- **Choosing a plugin.** A plugin node's `plugin` literal is a dropdown of installed plugins: instruments for `plugin_instrument`, everything else for `plugin_effect` (by the CLAP `instrument` feature). The pack scans the standard CLAP locations off the startup path and publishes the lists (`aura.plugins.instruments`, `aura.plugins.effects`).
- **Editing a plugin.** **Open plugin GUI** in Properties opens the plugin's own GUI on a separate editor instance configured from the node's `state` and `params`. Opening it again for the same node brings the open editor forward.
- **Edits flow into the node.** When the plugin reports a change (it marks its state dirty, asks for a parameter flush, or rescans values), the editor saves the plugin's state 300 ms after the last change, and once more on close if a change is pending. Each saved state becomes a node edit (`aura.plugins.edits`): `state` set to the new blob, `params` removed (the state holds every parameter). One editor session is one undo step in Lab.
- **Hearing edits.** The editor instance is not the one sounding: edits are heard on the next audition note or the next Play, which translate from the node's data.

Editors (`aura_clap::ClapHost::open_editor`) embed the plugin GUI in a host window where the CLAP main thread runs a window system and the plugin supports embedding, and ask the plugin for a floating window otherwise. Plugins without a GUI report that, and Lab shows the message.

## Channels

Plugins are adapted to the node's [audio layout](audio-layouts.md) (`LayoutStage`):

- `plugin_instrument` is stereo: the plugin's first two outputs, or a single output duplicated.
- `plugin_effect` is layout-polymorphic (`A: Audio`). Mono: the input feeds every plugin input channel and the output is the average of the plugin's outputs. Stereo: left/right feed the first two inputs (averaged into a single-input plugin) and the first two outputs are left/right.
- One plugin instance always processes every channel; plugin nodes never run as dual mono.
- Streaming stages run in stereo. Other ports receive silence and are ignored.

## CLAP backend

- `ClapHost` searches its extra paths, then `CLAP_PATH`, then the platform's standard locations (Linux `~/.clap`, `/usr/lib/clap`; Windows `%COMMONPROGRAMFILES%\CLAP`, `%LOCALAPPDATA%\Programs\Common\CLAP`; macOS `~/Library/Audio/Plug-Ins/CLAP`, `/Library/Audio/Plug-Ins/CLAP`). Bundles load lazily on first discovery or resolve and stay loaded for the host's life. Bundles that fail to load are reported, not fatal.
- Parameters are read through the CLAP params extension (a plugin without it has none). Aura declares the host params extension as a no-op: it sets parameters only from node data.
- Every instance's main-thread half runs on **one process-wide CLAP main thread** (frameworks such as JUCE assume one). Only the audio processor leaves it; dropping a processor deactivates and destroys its instance there. That thread also services plugins' `request_callback`.
- The CLAP main thread runs a `winit` event loop (X11 on Linux, so plugins can embed; it also pumps Win32 messages) when it can open a display, and a plain job loop otherwise (headless CI and containers), where only floating plugin GUIs open. macOS has no window system yet.
- Plugins run **in-process**.

## Non-goals / avoided approaches

- One plugin instance per note in streaming, except as the documented per-voice fallback
- Rendering or waiting on a plugin from the transport audio thread
- Depending on VST3 → CLAP adapters for other protocols; they are backends behind `Processor`
- Protocol-specific code in `aura_plugin_host`, `aura_imp`, or the translator
- Interpreting plugin state blobs
- Plugin-specific ports on plugin nodes (for now; see open questions)

## Open questions

- Parameter ports driven by signals (automation, plain values), per-note parameters (`NoteParams` → per-note modulation), and generated per-plugin node types; deferred while plugins are configured only by embedded data
- Reusing renders across plays (each play translates afresh and renders from the start)
- A "preparing" state in Imp Lab while a late start renders
- Out-of-process sandboxing
- Hearing GUI edits immediately (the editor would have to drive the sounding instance)
- Linux plugin GUIs that need the host's timer or file-descriptor support (JUCE on Linux); Windows is the verified path
- Aura audio beyond stereo
- Bounding cache memory for long renders (currently about 10.6 MB per plugin node per minute at 44.1 kHz)
