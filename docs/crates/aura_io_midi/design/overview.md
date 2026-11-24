# aura_io_midi — design

## Summary

MIDI input adapter ([live recording](../../../design/live-recording.md)). Lists input ports, opens one by name, and turns its messages into Aura performance terms (`aura_sequencing::capture::CapturedKind`) with the port's timestamp and channel. It is the only Aura code that reads MIDI bytes.

## Requirements

- `input_names()` lists the present input ports; `PortLister` abstracts it (`SystemPorts` is the midir implementation) so reconnect logic can be tested without hardware.
- `MidiInput::open(name, on_event)` opens the port whose name matches exactly and calls `on_event` on midir's thread with a `MidiInputEvent { timestamp_us, channel, kind }`. Dropping it closes the port.
- `parse_message(bytes)` is pure: note-on (velocity > 0) → `KeyDown` with velocity `v / 127`; note-off or note-on with velocity 0 → `KeyUp`; CC 64 ≥ 64 → `SustainDown`, < 64 → `SustainUp`; everything else is ignored.
- MIDI key numbers map directly to `Semitone` (A4 = 69 in both).
- `ChannelFilter` selects channels 1–16; an empty selection means every channel.
- Errors are typed (`MidiError`), never panics; an absent MIDI system or device is an error the caller reports, not a crash.

## Non-goals / avoided approaches

- MIDI as notation's source of truth, or storing MIDI
- Device policy (which port, reconnecting): callers own it (the Lab playback extension)
- Output, clock, sync, pitch bend, and other controllers

## See also

- Code: [`../../../../crates/aura_io_midi`](../../../../crates/aura_io_midi)
