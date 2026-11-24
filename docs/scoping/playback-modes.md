# Playback modes: motivation

Why Aura has two playback modes, **deterministic** and **streaming** (Aura's own terms), and why it ranks them the opposite way from traditional tools. Design: [`../design/playback-modes.md`](../design/playback-modes.md).

## Inverting the MIDI / DAW priority

Traditional synthesizers and DAWs, especially MIDI and MIDI-based tools, put streaming first. A note-on arrives, the instrument reacts, and the note-off arrives at some unknown later time. Offline rendering is mostly a faster run of that same reactive loop. Look-ahead is rare and bolted on.

Aura goes the other way. Its primary model is deterministic: any point in time can be sampled out of order, along with anything around it in the past or future. Instruments know each note's full length and the notes that follow. Streaming is supported, but as a graceful degradation of that model, not its foundation. Streaming output is not expected to sound identical to deterministic output.

Aura's model is also more declarative than MIDI-style technology. The same declarative data (notation, instrument graphs, envelope parameters) drives both modes; the modes differ in how that data is evaluated, not in what it says.

## The envelope example

A MIDI volume envelope decays independently of note length, and its release starts when the note ends. Aura's deterministic envelope is shaped by the actual note length: the release fits inside the note. Streaming mode can't know a note's length while it sounds, so it needs a fallback that uses the same envelope parameters. The fallback holds the sustain until note-off and then releases past it. This is the archetype for every streaming fallback: same declarative parameters, a degraded evaluation.

## Use cases driving streaming

1. **Instrument audition in the notation view.** When a user places, clicks, or drags a note in Imp Lab's piano roll, the note sounds immediately and for as long as it's held. The release time is genuinely unknown ahead of time.
2. **Live MIDI recording.** Incoming MIDI plays through streaming mode while it's captured. The MIDI data itself isn't stored; it's translated into Aura's deterministic notation model, so the recording becomes ordinary deterministic material afterwards. Settled for the first version: a take records from the playhead while the notation plays on the transport with a metronome and a one-bar count-in; timing stays raw (snapping is a later, separate edit); a take is added to the existing notes (overdub) as one undo step; one input at a time, chosen by port name in app-level settings with a channel filter, reconnecting on its own after an unplug. Design: [`../design/live-recording.md`](../design/live-recording.md).

## Shaping decisions

- Imp nodes stay the unit of meaning across modes. Imp graphs are translated into a runtime form that actually executes, and that runtime form is still a graph of nodes, optimized for execution instead of analysis and transformation. The mode influences that translation, so each node type owns its execution code for each mode, including node types that extensions provide.
- DSP functions shouldn't need two versions. Most nodes mean the same thing in either mode; only nodes that depend on note length, look-ahead, or the whole score need a streaming variant. A node type without one falls back to its deterministic implementation instead of failing.
