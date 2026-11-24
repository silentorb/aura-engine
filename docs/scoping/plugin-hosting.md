# Plugin hosting: motivation

Why Aura hosts third-party audio plugins, and why it does so through a protocol-neutral seam with CLAP first. Design: [`../design/plugin-hosting.md`](../design/plugin-hosting.md).

## Goal

Let Aura use third-party synthesizers and effects. Mature instruments and effects (wavetable synths, reverbs, compressors, …) would take years to reimplement as Aura DSP; hosting them widens Aura's palette immediately.

CLAP is the first protocol: it is open, has a clean threading model, and has a usable Rust host library (`clack-host`). Other protocols (VST3, possibly LV2) are expected later.

## The model mismatch

Plugin protocols are built for the streaming model Aura deliberately ranks second ([playback modes](playback-modes.md)):

| | Aura (deterministic) | Plugin (CLAP) |
|---|---|---|
| Unit | Pure `at(t) → f32`, seekable, any order | Stateful object, `process(block)`, forward only |
| Notes | Full score known up front, with lengths | Note-on/off events in time order |
| Polyphony | One graph instance per note | One instance, voices inside |
| Channels | Mono | Mostly stereo |
| Reproducibility | Bit-for-bit | Whatever the plugin does |
| Persistence | Declarative graph data | Opaque, plugin-versioned state blob |

A plugin's DSP state cannot be snapshotted (`state` saves presets, not filter history), so sampling time `t` means having run the plugin from a known start up to `t`. That is the central wrangling: a plugin becomes a *frozen region* of an otherwise pure graph, rendered forward once and then read like any sampler.

Streaming is the protocols' native model, so there the plugin runs nearly as-is; the work is on Aura's side, whose streaming runtime instantiates a whole voice graph per note.

## Advantages Aura's model brings

Because the score is fully known, deterministic rendering can place every note on its exact frame, compensate the plugin's reported latency perfectly, render its tail, and ask the plugin for offline-quality rendering. A streaming host can do none of that.

## Why a neutral seam instead of adapters

Mature adapters that load VST3 plugins *into* a CLAP host do not appear to exist; common tools (such as clap-wrapper) convert the other way, exposing a CLAP plugin as VST3. So later protocols are planned as further backends behind the same seam, not as CLAP adapters. A plugin someone has wrapped as CLAP works automatically, since it is just a CLAP plugin.

## Known limits accepted for now

- **Determinism weakens** to "as reproducible as the plugins in the graph": unseeded randomness, threading, or a different installed version change output.
- **Mono**: stereo plugins are folded to mono until Aura has multichannel audio. Stereo effects (reverb, chorus) lose their width; this is expected to be the first limit that matters.
- **In-process**: a crashing plugin takes the host with it; sandboxing is deferred.
- **Seeking cost**: a plugin region renders from the start; a late start in a long song with heavy plugins waits for the render.

## Phasing

1. Hosting seam, CLAP backend, deterministic render cache, streaming shared stages (audition and live MIDI through real synths), CLI and demo.
2. Plugin parameters as embedded node data. Settled at the start of this phase: keep it simple and support soft synths configured entirely by node data (saved state plus parameter values in plain units), with no plugin-specific ports. Imp ports carry no names or ranges, and large synths have hundreds of parameters, so parameter ports, signal automation, per-note parameters, and generated per-plugin node types wait until they are needed.
3. Real-time transport: render-ahead so `PlaybackEngine` never catches a plugin up on the audio thread. A play starting late in a long song waits while plugins render from the start; reusing renders across plays is left for when it hurts.
4. Imp Lab: plugin browser, plugin GUI windows, capturing state on edit. Settled at the start: Lab gains small generic node UI components rather than knowing plugins; GUI edits reach the node live as one undo step per editor session; edits are heard on the next note or Play, since the editor has its own instance.
