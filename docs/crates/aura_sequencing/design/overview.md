# aura_sequencing — design

## Summary

Musical notation types and musical-time utilities (semitones, scores, chords, tempo, time signature, note params).

## Requirements

- Own notation ontology: semitones, scale degrees, intervals, chords, keys — not MIDI as source of truth.
- Provide beat↔second conversion and score/event sorting helpers.
- Support extensible per-note parameters on `NoteEvent`.
- Provide `LiveNote` (a note still sounding, release stamped at note-off) and carry it in `ScoreSignal::live` for streaming mode ([playback modes](../../../design/playback-modes.md)).
- `capture` translates a captured performance (`CapturedEvent`: key down / up, sustain down / up, in notation seconds) into `NoteEvent`s by the [live recording](../../../design/live-recording.md#translation-rules) rules: raw timing, per-key pairing, sustain, held-at-stop, a minimum length. It is pure and knows nothing of MIDI.

## Non-goals / avoided approaches

- Procedural generators (belong in `aura_composition`)
- Sample-frame scheduling (belong in `aura_scheduler`)
- DSP / Imp translation

## See also

- [`../../../design/notation-synthesis.md`](../../../design/notation-synthesis.md)
- Code: [`../../../../crates/aura_sequencing`](../../../../crates/aura_sequencing)
