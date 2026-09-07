# aura_scheduler — design

## Summary

Offline beat→frame scheduling and lookahead via `ScheduleContext`.

## Requirements

- Convert musical scores into sample-frame timelines.
- Provide future-aware schedule context for offline instrumentation.
- Live CPAL dispatch remains deferred; this crate owns the offline path.

## Non-goals / avoided approaches

- Real-time MIDI ingest as a v1 requirement
- Owning Imp translation or WAV I/O

## See also

- [`../../../design/notation-synthesis.md`](../../../design/notation-synthesis.md)
- Code: [`../../../../crates/aura_scheduler`](../../../../crates/aura_scheduler)
