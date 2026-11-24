# Live recording

Live MIDI input plays through [streaming mode](playback-modes.md) and is recorded into ordinary deterministic notation. The MIDI itself is never stored. Motivation: [`../scoping/playback-modes.md`](../scoping/playback-modes.md).

## MIDI is an adapter

[Notation's ontology](notation-synthesis.md#ontology) is Aura's own. MIDI exists only at the input edge:

```
MIDI device ─midir─▶ aura_io_midi ─▶ Lab playback extension ─┬─▶ streaming voices (monitoring)
                     (bytes → Aura terms)                     └─▶ take ─▶ aura_sequencing::capture ─▶ notation notes (beats)
```

- `aura_io_midi` is the only code that reads MIDI bytes. It produces Aura terms (`CapturedKind`: semitone, velocity 0–1, sustain down/up), each with the port's timestamp and channel.
- MIDI key numbers are Aura semitones (both put A4 at 69). Velocity is `v / 127`.
- Only notes and the sustain pedal (CC 64, ≥ 64 = down) are read. Note-on with velocity 0 is a note-off.
- Translating a take into notation is a pure function (`aura_sequencing::capture::notes_from_capture`), testable without hardware.
- Imp Lab never sees MIDI. It sees notation notes, setting values, and topic values.

## Translation rules

Times are seconds on the notation's timeline (beat 0 = 0 s at the notation tempo). Timing is **raw**: nothing is snapped to a grid.

| Situation | Result |
|---|---|
| Key-down then key-up, same key | One note: start = key-down, length = key-up − key-down, in beats at the notation tempo |
| Key-down for a key already sounding | The earlier note ends there; a new note starts |
| Chords / overlapping keys | Independent notes, paired per key |
| Key released while the sustain pedal is down | Sounds until pedal-up (monitoring does the same) |
| Key or pedal still held when the take stops | The note ends at the stop |
| Key-up without a key-down (pressed before capture began) | Ignored |
| Notes played during the count-in | Kept at their real position before the playhead; starts clamped at beat 0 |
| Very short notes | At least `MIN_DURATION_BEATS` (1/32 beat) |
| Velocity | Carried into the note's velocity |
| MIDI channels | Filtered by the input's channel setting before monitoring and capture; allowed channels merge into one take |

## Devices

- One input is active at a time. It is chosen by **port name**, never by index (indexes change when devices are plugged in or out).
- Imp Lab stores the configuration as two extension settings owned by the playback service: `aura.midi.input` (port name, `""` = none) and `aura.midi.channels` (the allowed channels, `"1"`–`"16"`; all sixteen = omni). The service receives them in a `settings` event at startup and on change. Lab stores the values; Aura owns the device.
- Connection lifecycle:

  | State | Meaning | Status |
  |---|---|---|
  | Off | No device configured | `MIDI: off` (`off`) |
  | Connected | The configured port is present and open | `MIDI: ‹name›`, plus the last note played (`ok`) |
  | Waiting | Configured but not present (unplugged, or held exclusively by another app) | `MIDI: ‹name› (disconnected)` (`warn`) |
  | Error | Opening failed for another reason | the error (`error`) |

- midir has no hot-plug notification, so the extension rescans port names about every 1.5 s, privately. It reconnects when the configured name reappears and moves to Waiting when it vanishes.
- Lab is never polled for this. The extension publishes the topics `aura.midi.ports` (choice options) and `aura.midi.status` (a status item value) only when they change, with activity throttled to about 10 Hz.
- On a disconnect every held note from the device is released, both in monitoring and in a running take (the take keeps those notes, ended at the disconnect). A disconnect does not stop the take.

## Monitoring

- While the input is connected, its notes play through the streaming path (`StreamingProgram` → `VoiceMixer` → `StreamingOutput`), whether or not a take is recording. The MIDI callback thread starts and releases voices directly; the UI is not on the path.
- Monitoring uses the same instrument as note audition (`sine_note_voice`). MIDI voices use ids from `MIDI_VOICE_ID_BASE` up, so they never release audition voices.
- Latency is the MIDI driver plus one output buffer.

## Recording a take

- **Start** takes the notation, the playhead, and whether the click is on. It starts deterministic transport playback of the notation with no auto-stop. With the click on, playback starts one bar before the playhead (count-in), and a metronome plays on the notation's bar grid.
- **Metronome.** The click is deterministic data: `aura_composition::click_bar` (one note per counted beat of the time signature, downbeat accented) voiced by the Lab pack's `metronome_click` instrument graph. It is not a special streaming voice.
- **Clock.** The take pairs the input's timestamp clock with the transport's notation time at the take's first event. Later events follow the input timestamps, which avoids the jitter of reading the transport clock per event.
- **While recording**, the extension publishes the take on the `aura.capture.take` topic: `{recording, countIn, notes}`, with held notes at their duration so far, about 30 times a second while notes grow.
- **Stop** stops the transport, ends held notes at the stop time, translates the take, and returns its notes. Stopping the transport any other way also ends the take; its notes wait for the next stop request.
- In Imp Lab the take is added to the notation (overdub) as one undoable edit. See Imp Lab's `docs/features/extensions.md` for the live note capture protocol.

## Non-goals / avoided approaches

- Storing MIDI, or MIDI as the notation's source of truth
- Quantizing on capture (snapping is a separate, later edit)
- Reading MIDI bytes outside `aura_io_midi`
- Lab polling extensions for device or take state (extensions publish topics)
- Several inputs at once; per-device transpose or velocity curves; MIDI output, clock, or sync
- Pitch bend, mod wheel, aftertouch, and other controllers

## Open questions

- Output-latency compensation of recorded timing. You play against a click heard one output buffer (plus driver latency) late, so raw takes can sit slightly late.
- Monitoring and the transport are separate device streams, so the click and the monitored notes may differ slightly in latency.
- Choosing the monitoring instrument per notation.
