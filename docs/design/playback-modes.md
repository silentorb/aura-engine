# Playback modes

Aura renders audio in one of two modes. **Deterministic** is primary; **streaming** is a graceful degradation of it. Motivation: [`../scoping/playback-modes.md`](../scoping/playback-modes.md). Related: [`imp-execution.md`](imp-execution.md), [`notation-synthesis.md`](notation-synthesis.md).

## Modes

| | Deterministic (primary) | Streaming (fallback) |
|---|---|---|
| Sampling | Any time, any order; may look into the past and future | Time only moves forward; nothing about the future is known |
| Note knowledge | The full score: every note's start and length, and the notes that follow | One **live note** per voice: pitch, velocity, params, onset; its release time is known only once note-off happens |
| Instancing | A graph samples the whole score | **One graph instance per note** ([Rule 1](notation-synthesis.md#rule-1-per-note-instances)) |
| Output | WAV export and transport playback; bit-for-bit reproducible | Live device output (note audition; later live MIDI) |

- Deterministic mode is the reference. Where the two modes disagree, deterministic is correct by definition.
- Streaming output need not sound identical to deterministic output.
- Streaming is reproducible for an identical sequence of events with identical timing, but makes no other promise.
- The same declarative data (notation, Imp graphs, node parameters) drives both modes.

## Node implementations

An Imp graph is translated once into a runtime form that executes: block signals that render ranges of frames ([execution model](imp-execution.md)), in both modes. Each runtime node comes from an Imp node, and its code belongs to that Imp node's **type**.

- Every node type **must** have a deterministic implementation.
- A node type **may** have a streaming implementation. In streaming mode the translator builds it when it exists, and the deterministic implementation otherwise.
- Translating for streaming never fails because of the mode. A graph that translates deterministically also translates for streaming.
- Nodes in the Imp graph never check a mode flag while rendering. The mode is applied once, at translation, by choosing which implementation to build.
- State follows the mode ([node state](imp-execution.md#node-state-one-rule-per-mode)): a deterministic implementation's state is nonessential (any range renders identically), while a streaming implementation may keep history, since streaming only renders consecutive blocks forward. A history-dependent algorithm (a recursive filter, a phase accumulator) is therefore a streaming implementation.
- Mode-agnostic node types (`sine`, `add`, `multiply`, noise, `bandlimited_saw`, `semitone_to_hz`, constants, `time`, …) have one implementation, used in both modes. DSP is not duplicated per mode.
- Node types **register** their implementations (`aura_integration`, `translate/nodes.rs`). When the registry is built, a missing streaming implementation is filled with the deterministic one, so translation picks a ready cell instead of falling back. Layout adds a second dimension: see the [implementation matrix](audio-layouts.md#implementation-matrix) (mono/stereo × deterministic/streaming).
- A node type supplied by an extension follows the same rule: it registers its deterministic implementation and, optionally, its streaming (and stereo) ones.

### Live notes travel as scores

In streaming mode the live note is carried inside the ordinary score value (`ScoreSignal::live`): a score holding the note as it stood at note-on, plus the live note itself.

- Every score consumer receives a valid score, so deterministic implementations accept it unchanged.
- Streaming implementations read the live note.
- A note-reading streaming implementation acts on the live note only when its score input carries one. A fixed score (from `arpeggio`, `bass_line`, `drum_grid`, a loop of them, …) is fully known in either mode, so those nodes use their deterministic implementations and keep full look-ahead.
- A score consumer without a streaming implementation (currently `loop`) sees a live note as its note-on snapshot. That's a degraded result, not an error. Adding a streaming implementation is the fix when it matters.

### Streaming implementations

| Node type | Streaming behavior |
|---|---|
| notation node | The voice's live note |
| `note_at_time` | `semitone`, `velocity` from the live note; `note_start` = onset; `active` = held; `note_duration` = **duration so far** (grows while held, fixed at its final length after release) |
| `note_param_at_time` | The live note's params |
| `linear_adsr` | **Gated envelope** from the same attack / decay / sustain / release: attack → decay → sustain while held; at note-off, a linear release from the current level that rings past note-off. `note_duration` is not read. Applies when the envelope's `elapsed` or `note_duration` input depends on a notation node; an envelope timed by a fixed score keeps its deterministic implementation. |
| `note_envelope` | Gated envelope (default ADSR) |
| `poly_sine_score` | The live note's sine × gated envelope × velocity |
| `plugin_instrument`, `plugin_effect` | Per-voice fallback: the voice's own plugin instance (an instrument plays the live note). Where the output is a plugin chain, `StreamingProgram::open` shares one instance per plugin across all voices instead ([plugin hosting](plugin-hosting.md#streaming-mode-shared-instances)). |

The gated envelope is the archetypal fallback. Deterministically, `LinearAdsr` places the release inside the note's known length, so the envelope ends with the note. In streaming the length is unknown, so the same parameters are evaluated against the note-off time instead (`LinearAdsr::gated_value_at`).

## Streaming runtime

- **Program** — `StreamingProgram` type-checks and flattens a voice graph once, then instantiates a fresh runtime graph per note, bound to that note's `LiveNote` (voice-local time, onset `t = 0`). `StreamingProgram::open` builds the whole pipeline: a `StreamingControl` for notes and the `BlockSource` to play, sharing plugin instances where the graph allows.
- **Voice** — `StreamVoice` (`aura_render`) runs one instance on its own clock, rendering consecutive blocks from frame 0. Note-off stamps the release on the live note; it takes effect from the next block. A released voice retires once it has been silent briefly, or after a hard tail limit.
- **Mixer** — `VoiceMixer` (`aura_render`) is a `BlockSource`. Blocks are stereo frames; a mono voice sounds on both channels, a stereo voice keeps its channels. It takes note-on / note-off / all-off commands from a control thread without blocking, caps polyphony (starting another voice releases the oldest), and sums voices block by block. It neither allocates nor frees on the audio thread: voices sit in preallocated slots, and finished voices are handed back to the control thread to be dropped.
- **Output** — `StreamingOutput` (`aura_io_cpal`) is a persistent device stream that pulls from a `BlockSource`. It runs independently of deterministic `PlaybackEngine` transport playback; both may sound at once.

## Uses

- **Note audition** in Imp Lab's notation view: the Lab playback extension (`aura_imp_lab_playback`) opens the pack's `sine_note_voice` as a streaming pipeline and turns `note_on` / `note_off` service events into its notes.
- **Live MIDI recording** ([live recording](live-recording.md)): a MIDI input plays through streaming mode (the same voices as audition) while a take is captured; the take is translated into deterministic notation, not stored as MIDI.

## Non-goals / avoided approaches

- Streaming as the primary model, or MIDI-style reactive data as the source of truth
- Duplicating mode-agnostic DSP per mode
- Mode-specific translation errors (a node type without a streaming implementation falls back; it never fails)
- Guessed defaults for unknown future values; a streaming implementation states what it knows (for example, duration so far)
- Requiring streaming output to match deterministic output

## Open questions

- Choosing an instrument per notation for audition and transport (audition currently uses `sine_note_voice`)
- One device output mixing both modes
- Output-latency compensation for recorded takes ([live recording](live-recording.md#open-questions))
