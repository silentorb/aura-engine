# aura_sequencing — design

## Summary

Musical notation types and musical-time utilities (semitones, scores, chords, tempo, time signature, note params).

## Requirements

- Own notation ontology: semitones, scale degrees, intervals, chords, keys — not MIDI as source of truth.
- Provide beat↔second conversion and score/event sorting helpers.
- Support extensible per-note parameters on `NoteEvent`.

## Non-goals / avoided approaches

- Procedural generators (belong in `aura_composition`)
- Sample-frame scheduling (belong in `aura_scheduler`)
- DSP / Imp translation

## See also

- [`../../../design/notation-synthesis.md`](../../../design/notation-synthesis.md)
- Code: [`../../../../crates/aura_sequencing`](../../../../crates/aura_sequencing)
