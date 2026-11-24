//! MIDI input adapter for Aura.
//!
//! This is the only Aura code that knows MIDI bytes. It lists input ports, opens one by name, and
//! turns its messages into Aura performance terms ([`CapturedKind`]: semitone, velocity 0–1,
//! sustain) with the port's timestamp and channel. MIDI is never the notation's source of truth;
//! captured events are translated into notation by `aura_sequencing::capture`.

use aura_sequencing::capture::CapturedKind;
use aura_sequencing::Semitone;
use midir::{Ignore, MidiInputConnection};
use thiserror::Error;

const CLIENT_NAME: &str = "Aura";
/// MIDI control change number of the sustain (damper) pedal.
const SUSTAIN_CONTROLLER: u8 = 64;

#[derive(Debug, Error)]
pub enum MidiError {
    #[error("MIDI is unavailable: {0}")]
    Init(String),
    #[error("MIDI input `{0}` is not present")]
    NotFound(String),
    #[error("MIDI input `{name}` could not be opened: {message}")]
    Connect { name: String, message: String },
}

/// One MIDI input message, in Aura terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiInputEvent {
    /// Microseconds on the port's clock (monotonic within one connection).
    pub timestamp_us: u64,
    /// MIDI channel, 1–16.
    pub channel: u8,
    pub kind: CapturedKind,
}

/// Parses one MIDI message into `(channel 1–16, kind)`.
///
/// Note-on with velocity 0 is a note-off. Only notes and the sustain pedal (CC 64; ≥ 64 = down)
/// are read; everything else is `None`.
pub fn parse_message(bytes: &[u8]) -> Option<(u8, CapturedKind)> {
    let (&status, data) = bytes.split_first()?;
    let channel = (status & 0x0F) + 1;
    let semitone = |key: u8| Semitone(i16::from(key & 0x7F));
    let kind = match (status & 0xF0, data) {
        (0x90, [key, velocity, ..]) if *velocity > 0 => CapturedKind::KeyDown {
            semitone: semitone(*key),
            velocity: f32::from(*velocity & 0x7F) / 127.0,
        },
        (0x90, [key, _, ..]) | (0x80, [key, _, ..]) => CapturedKind::KeyUp {
            semitone: semitone(*key),
        },
        (0xB0, [SUSTAIN_CONTROLLER, value, ..]) => {
            if *value >= 64 {
                CapturedKind::SustainDown
            } else {
                CapturedKind::SustainUp
            }
        }
        _ => return None,
    };
    Some((channel, kind))
}

/// Which MIDI channels (1–16) an input listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelFilter(u16);

impl ChannelFilter {
    /// Every channel (omni).
    pub const ALL: Self = Self(u16::MAX);

    /// Listens to `channels` (1–16; others are ignored). An empty selection means every
    /// channel, so an input is never silently deaf.
    pub fn from_channels(channels: &[u8]) -> Self {
        let mask = channels
            .iter()
            .filter(|c| (1..=16).contains(*c))
            .fold(0_u16, |mask, c| mask | 1 << (c - 1));
        if mask == 0 {
            Self::ALL
        } else {
            Self(mask)
        }
    }

    pub fn allows(self, channel: u8) -> bool {
        (1..=16).contains(&channel) && self.0 & (1 << (channel - 1)) != 0
    }
}

impl Default for ChannelFilter {
    fn default() -> Self {
        Self::ALL
    }
}

/// Lists MIDI input port names. Implemented over midir by [`SystemPorts`]; tests use fakes.
pub trait PortLister: Send + Sync {
    fn names(&self) -> Result<Vec<String>, MidiError>;
}

/// The system's MIDI input ports.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPorts;

impl PortLister for SystemPorts {
    fn names(&self) -> Result<Vec<String>, MidiError> {
        input_names()
    }
}

fn midi_input() -> Result<midir::MidiInput, MidiError> {
    let mut input =
        midir::MidiInput::new(CLIENT_NAME).map_err(|e| MidiError::Init(e.to_string()))?;
    input.ignore(Ignore::All);
    Ok(input)
}

/// Names of the MIDI input ports present now, in system order.
pub fn input_names() -> Result<Vec<String>, MidiError> {
    let input = midi_input()?;
    Ok(input
        .ports()
        .iter()
        .filter_map(|port| input.port_name(port).ok())
        .collect())
}

/// An open MIDI input. Messages arrive on a midir thread until this is dropped.
pub struct MidiInput {
    name: String,
    _connection: MidiInputConnection<()>,
}

impl MidiInput {
    /// Opens the input named `name` (exact match). `on_event` runs on midir's thread for each
    /// note or sustain message.
    pub fn open(
        name: &str,
        mut on_event: impl FnMut(MidiInputEvent) + Send + 'static,
    ) -> Result<Self, MidiError> {
        let input = midi_input()?;
        let port = input
            .ports()
            .into_iter()
            .find(|port| input.port_name(port).is_ok_and(|n| n == name))
            .ok_or_else(|| MidiError::NotFound(name.to_string()))?;
        let connection = input
            .connect(
                &port,
                "aura-input",
                move |timestamp_us, bytes, _| {
                    if let Some((channel, kind)) = parse_message(bytes) {
                        on_event(MidiInputEvent {
                            timestamp_us,
                            channel,
                            kind,
                        });
                    }
                },
                (),
            )
            .map_err(|e| MidiError::Connect {
                name: name.to_string(),
                message: e.to_string(),
            })?;
        Ok(Self {
            name: name.to_string(),
            _connection: connection,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_and_off_on_every_channel() {
        for channel in 0..16_u8 {
            assert_eq!(
                parse_message(&[0x90 | channel, 60, 127]),
                Some((
                    channel + 1,
                    CapturedKind::KeyDown {
                        semitone: Semitone(60),
                        velocity: 1.0
                    }
                ))
            );
            assert_eq!(
                parse_message(&[0x80 | channel, 60, 64]),
                Some((
                    channel + 1,
                    CapturedKind::KeyUp {
                        semitone: Semitone(60)
                    }
                ))
            );
        }
    }

    #[test]
    fn note_on_with_zero_velocity_is_note_off() {
        assert_eq!(
            parse_message(&[0x91, 64, 0]),
            Some((
                2,
                CapturedKind::KeyUp {
                    semitone: Semitone(64)
                }
            ))
        );
    }

    #[test]
    fn velocity_scales_to_unit_range() {
        let Some((_, CapturedKind::KeyDown { velocity, .. })) = parse_message(&[0x90, 60, 64])
        else {
            panic!("key down");
        };
        assert!((velocity - 64.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn sustain_pedal_threshold() {
        assert_eq!(
            parse_message(&[0xB0, 64, 127]),
            Some((1, CapturedKind::SustainDown))
        );
        assert_eq!(
            parse_message(&[0xB0, 64, 64]),
            Some((1, CapturedKind::SustainDown))
        );
        assert_eq!(
            parse_message(&[0xB0, 64, 63]),
            Some((1, CapturedKind::SustainUp))
        );
    }

    #[test]
    fn other_messages_are_ignored() {
        assert_eq!(parse_message(&[]), None);
        assert_eq!(parse_message(&[0x90, 60]), None, "truncated");
        assert_eq!(parse_message(&[0xB0, 1, 100]), None, "mod wheel");
        assert_eq!(parse_message(&[0xE0, 0, 64]), None, "pitch bend");
        assert_eq!(parse_message(&[0xD0, 40]), None, "aftertouch");
        assert_eq!(parse_message(&[0xF8]), None, "clock");
    }

    #[test]
    fn channel_filter() {
        assert!((1..=16).all(|c| ChannelFilter::ALL.allows(c)));
        let only = ChannelFilter::from_channels(&[1, 10]);
        assert!(only.allows(1) && only.allows(10));
        assert!(!only.allows(2) && !only.allows(16));
        assert!(!only.allows(0) && !only.allows(17));
        assert_eq!(ChannelFilter::from_channels(&[]), ChannelFilter::ALL);
        assert_eq!(ChannelFilter::from_channels(&[0, 17]), ChannelFilter::ALL);
    }
}
