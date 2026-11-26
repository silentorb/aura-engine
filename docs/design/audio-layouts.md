# Audio layouts

Aura signals are **mono** or **stereo**. Motivation: [`../scoping/stereo.md`](../scoping/stereo.md). Related: [`imp-execution.md`](imp-execution.md), [`playback-modes.md`](playback-modes.md), [`plugin-hosting.md`](plugin-hosting.md).

## Types

| Signal type | Runtime | Meaning |
|---|---|---|
| `audio_mono` | `Signal`: a range of frames → one buffer | One channel |
| `audio_stereo` | `StereoSignal`: a range of frames → left and right buffers (planar) | Two channels |

- **`Audio`** is an Imp **union type**: `Audio = audio_mono | audio_stereo`, an Aura type definition shipped in the `aura.types` library (`aura_imp::types`). It is a real type, not just notation: it can bound a type parameter (`A: Audio`) or type a port directly. Layout-polymorphic node types use the bound, because a port typed plain `Audio` accepts either layout but loses "the output layout follows the input". Translation decides layouts statically, so a node whose layout resolves to the whole union (fed by an `Audio`-typed port) is a translation error (`UnresolvedLayout`); state its layout with `typeArgs`.
- Control, time, score and other signals have no layout.

## Layout-polymorphic node types

A node type generic over `A: Audio` is **layout-polymorphic**: its audio ports are typed `A`, and Imp inference resolves `A` from the graph. Non-audio ports keep their own types.

- Inputs bound to the same parameter must agree. A mono and a stereo signal into one `A` is an Imp type error (`type parameter A is audio_mono (from input a) but input b receives audio_stereo`); there is no implicit up-mix.
- A parameter that nothing in the graph determines (for example `add` of constant `parameter`s) is unresolved; the node states its layout with `typeArgs`.
- Graph-backed definitions may be generic too (`echo<A: Audio>(audio: A) → A`); each use site resolves its own layout.

| Node type | Signature |
|---|---|
| `multiply` | `(a: A, b: control) → A` |
| `add` | `(inputs: A…) → A` |
| `plugin_effect` | `(audio: A) → A` |

## Fixed-layout node types

A node type may be exclusively mono or stereo. Sources stay mono (`sine`, noise, saws, envelopes, `poly_sine_score`). Converters make mixed workflows explicit:

| Node type | Signature |
|---|---|
| `pan` | `(audio: audio_mono, position: control) → audio_stereo` — equal power, `position` in `[-1, 1]` (center −3 dB per side) |
| `stereo_to_mono` | `audio_stereo → audio_mono` — average |
| `stereo_join` | `(left, right: audio_mono) → audio_stereo` |
| `stereo_split` | `audio_stereo → left, right: audio_mono` |
| `plugin_instrument` | `→ audio_stereo` — a mono plugin's output is duplicated |

## Implementation matrix

Every node type registers its implementations in `aura_integration` ([`translate/nodes.rs`](../../crates/aura_integration/src/translate/nodes.rs)). When the registry is built, each registration is **finalized** into a complete matrix, so translation reads one cell and never falls back:

| | deterministic | streaming |
|---|---|---|
| **mono** | required | defaults to mono deterministic |
| **stereo** | defaults to **dual mono** of mono deterministic | defaults to the stereo deterministic implementation when one is given, else dual mono of mono streaming |

- Only layout-polymorphic node types have a stereo row. A fixed-layout node type has the mono row only (its two cells).
- The translator resolves each layout-polymorphic node's layout once per graph from Imp's inferred type args, then builds the cell for (layout, mode).
- Implementations are translate-time builders; "prepared at load" means the dual-mono wrapper exists from registration and only composes signals at translation.

### Dual mono

The dual-mono wrapper builds the mono implementation once per channel (one instance per channel, so a streaming implementation keeps separate history per channel) and pairs the results.

- Each pass sees the pass's channel of every stereo input and the same non-audio inputs.
- Inputs are translated **once** and shared by both passes ([shared outputs](imp-execution.md#shared-outputs)); upstream nodes (including plugin instances) are not built twice.
- A shared stereo input renders once per block for both channel views (`StereoChannel` over `SharedStereo`), so a chain of dual-mono nodes costs about twice its mono cost, not 2^N.
- Stateful node types (plugins) never take the dual-mono default; they register a true stereo implementation so one instance processes both channels.

## Outputs

The graph output's layout decides what sinks receive:

- **WAV:** one or two channels (`Pcm { layout, samples }`, interleaved).
- **Device playback** (`aura_io_cpal`): stereo frames; a mono device gets their average, a device with more channels gets left/right on its first two and silence on the rest. A mono graph plays the same on both channels.
- **Streaming:** voices may be mono or stereo; `BlockSource` blocks are stereo frames, and the voice mixer, plugin stages and device output are stereo internally.

## Non-goals / avoided approaches

- Implicit up-mix of mono into stereo inputs, or mixed layouts resolving by guess
- Layout flags read while rendering; layout is resolved once, at translation
- Dual mono for plugins (two plugin instances per stereo signal)
- More than two channels for now (a later layout is another `Audio` member)

## Deferred

- **Join inference:** `(A, A) → A` resolving to stereo when any input is stereo, with up-mix insertion (opt-in per type parameter; up-mix policy TBD)
- Resolved layout shown on generic ports in Imp Lab
