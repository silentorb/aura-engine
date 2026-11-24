//! Live note audition: streaming-mode voices started and released by Lab service events.
//!
//! The audition instrument (`sine_note_voice` from the Lab pack) is opened as a streaming
//! pipeline ([`StreamingProgram::open`]): each `note_on` starts a voice for one live note (or a
//! note on a shared plugin instance, when the instrument is a plugin chain); `note_off` releases
//! it so it rings out. The pipeline plays on a persistent [`StreamingOutput`], independent of
//! deterministic transport playback.

use crate::{playback_registry, PlaybackServiceError, SAMPLE_RATE};
use aura_imp::{graph_from_json_str, NOTATION_NODE_TYPE};
use aura_integration::{StreamingControl, StreamingProgram};
use aura_io_cpal::StreamingOutput;
use aura_plugin_host::PluginResolver;
use aura_sequencing::{LiveNote, NoteParams, Semitone};
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};

/// Instrument graph type that auditions notes (Lab pack `catalogs/instruments.json`).
pub const AUDITION_INSTRUMENT: &str = "sine_note_voice";

/// A Lab service event for live audition.
#[derive(Debug, Clone, PartialEq)]
pub enum AuditionEvent {
    NoteOn {
        id: u64,
        semitone: i16,
        velocity: f32,
    },
    NoteOff {
        id: u64,
    },
    AllNotesOff,
}

/// Parses `{"type":"note_on","id":1,"semitone":60,"velocity":1}`, `{"type":"note_off","id":1}`,
/// or `{"type":"all_notes_off"}`.
pub fn parse_event(json: &str) -> Result<AuditionEvent, PlaybackServiceError> {
    let bad = |message: &str| PlaybackServiceError::Event(message.to_string());
    let value: Value = serde_json::from_str(json).map_err(|e| bad(&e.to_string()))?;
    let id = || {
        value
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| bad("event needs a non-negative integer `id`"))
    };
    match value.get("type").and_then(Value::as_str) {
        Some("note_on") => {
            let semitone = value
                .get("semitone")
                .and_then(Value::as_i64)
                .and_then(|s| i16::try_from(s).ok())
                .ok_or_else(|| bad("note_on needs an integer `semitone`"))?;
            let velocity = value
                .get("velocity")
                .and_then(Value::as_f64)
                .map_or(1.0, |v| v.clamp(0.0, 1.0) as f32);
            Ok(AuditionEvent::NoteOn {
                id: id()?,
                semitone,
                velocity,
            })
        }
        Some("note_off") => Ok(AuditionEvent::NoteOff { id: id()? }),
        Some("all_notes_off") => Ok(AuditionEvent::AllNotesOff),
        Some(other) => Err(bad(&format!("unknown event type `{other}`"))),
        None => Err(bad("event needs a string `type`")),
    }
}

/// The audition graph: a notation node (the live note) through the audition instrument.
fn audition_graph_json() -> String {
    format!(
        r#"{{
          "nodes": {{
            "note": {{ "id": "note", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }},
            "time": {{ "id": "time", "type": "time", "inputs": {{}} }},
            "voice": {{ "id": "voice", "type": "{AUDITION_INSTRUMENT}", "inputs": {{}} }},
            "out": {{ "id": "out", "type": "output", "inputs": {{}}, "title": "out" }}
          }},
          "edges": {{
            "score": {{ "id": "score", "from": {{ "node": "note", "port": "score" }}, "to": {{ "node": "voice", "port": "score" }} }},
            "time": {{ "id": "time", "from": {{ "node": "time", "port": "time" }}, "to": {{ "node": "voice", "port": "time" }} }},
            "audio": {{ "id": "audio", "from": {{ "node": "voice", "port": "audio" }}, "to": {{ "node": "out", "port": "value" }} }}
          }}
        }}"#
    )
}

/// Prepares the audition instrument for streaming with the Lab pack's libraries.
pub fn audition_program() -> Result<StreamingProgram, PlaybackServiceError> {
    let graph = graph_from_json_str(&audition_graph_json())
        .map_err(|e| PlaybackServiceError::GraphJson(e.to_string()))?;
    let registry = playback_registry(None)?;
    let plugins: Arc<dyn PluginResolver> = crate::plugin_host().clone();
    StreamingProgram::prepare_with_plugins(&graph, &registry, SAMPLE_RATE, Some(plugins))
        .map_err(|e| PlaybackServiceError::Translate(e.to_string()))
}

/// A new live note for audition.
pub fn audition_note(semitone: i16, velocity: f32) -> Arc<LiveNote> {
    Arc::new(LiveNote::new(
        Semitone(semitone),
        velocity,
        NoteParams::new(),
    ))
}

struct Audition {
    control: StreamingControl,
    _output: StreamingOutput,
}

/// Ids at or above this belong to MIDI monitoring, below it to UI audition, so the two never
/// release each other's voices.
pub const MIDI_VOICE_ID_BASE: u64 = 1 << 40;

/// Starts a streaming voice under `id`, opening the streaming output on first use.
pub(crate) fn voice_on(id: u64, semitone: i16, velocity: f32) -> Result<(), PlaybackServiceError> {
    let mut guard = audition()
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    if guard.is_none() {
        *guard = Some(Audition::open()?);
    }
    let Some(audition) = guard.as_ref() else {
        return Ok(());
    };
    audition
        .control
        .note_on(id, audition_note(semitone, velocity))
        .map_err(|e| PlaybackServiceError::Playback(e.to_string()))
}

/// Releases the streaming voice under `id` (nothing sounds before the first note-on).
pub(crate) fn voice_off(id: u64) -> Result<(), PlaybackServiceError> {
    let guard = audition()
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    match guard.as_ref() {
        Some(audition) => audition
            .control
            .note_off(id)
            .map_err(|e| PlaybackServiceError::Playback(e.to_string())),
        None => Ok(()),
    }
}

/// Drops finished voices off the audio thread.
pub(crate) fn reclaim_voices() {
    if let Ok(guard) = audition().try_lock() {
        if let Some(audition) = guard.as_ref() {
            audition.control.reclaim();
        }
    }
}

impl Audition {
    fn open() -> Result<Self, PlaybackServiceError> {
        let (control, source) = audition_program()?
            .open()
            .map_err(|e| PlaybackServiceError::Translate(e.to_string()))?;
        let output = StreamingOutput::open(source, SAMPLE_RATE)
            .map_err(|e| PlaybackServiceError::Playback(e.to_string()))?;
        Ok(Self {
            control,
            _output: output,
        })
    }
}

fn audition() -> &'static Mutex<Option<Audition>> {
    static AUDITION: OnceLock<Mutex<Option<Audition>>> = OnceLock::new();
    AUDITION.get_or_init(|| Mutex::new(None))
}

/// Applies one audition event. The streaming output opens on the first `note_on`.
pub fn handle_event(json: &str) -> Result<(), PlaybackServiceError> {
    let event = parse_event(json)?;
    let mut guard = audition()
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    let sent = match event {
        AuditionEvent::NoteOn {
            id,
            semitone,
            velocity,
        } => {
            if guard.is_none() {
                *guard = Some(Audition::open()?);
            }
            let Some(audition) = guard.as_ref() else {
                return Ok(());
            };
            audition.control.note_on(id, audition_note(semitone, velocity))
        }
        // Nothing sounds before the first note-on.
        AuditionEvent::NoteOff { id } => match guard.as_ref() {
            Some(audition) => audition.control.note_off(id),
            None => Ok(()),
        },
        AuditionEvent::AllNotesOff => match guard.as_ref() {
            Some(audition) => audition.control.all_off(),
            None => Ok(()),
        },
    };
    sent.map_err(|e| PlaybackServiceError::Playback(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_events() {
        assert_eq!(
            parse_event(r#"{"type":"note_on","id":3,"semitone":64,"velocity":0.5}"#).unwrap(),
            AuditionEvent::NoteOn {
                id: 3,
                semitone: 64,
                velocity: 0.5
            }
        );
        assert_eq!(
            parse_event(r#"{"type":"note_on","id":1,"semitone":60}"#).unwrap(),
            AuditionEvent::NoteOn {
                id: 1,
                semitone: 60,
                velocity: 1.0
            }
        );
        assert_eq!(
            parse_event(r#"{"type":"note_off","id":3}"#).unwrap(),
            AuditionEvent::NoteOff { id: 3 }
        );
        assert_eq!(
            parse_event(r#"{"type":"all_notes_off"}"#).unwrap(),
            AuditionEvent::AllNotesOff
        );
        assert!(parse_event(r#"{"type":"note_on","id":1}"#).is_err());
        assert!(parse_event(r#"{"type":"bend","id":1}"#).is_err());
        assert!(parse_event("not json").is_err());
    }

    #[test]
    fn audition_instrument_streams_from_the_pack_without_a_device() {
        crate::tests::use_pack_libraries();
        let program = audition_program().expect("audition program");
        let (control, mut source) = program.open().expect("open");
        control.note_on(1, audition_note(69, 1.0)).unwrap();
        let mut block = vec![0.0; 4410];
        source.render_block(&mut block);
        let peak = block.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.5, "audible while held, peak {peak}");

        control.note_off(1).unwrap();
        for _ in 0..4 {
            source.render_block(&mut block);
        }
        assert!(
            block.iter().all(|s| s.abs() < 1e-4),
            "voice rings out after its release"
        );
        assert!(control.reclaim() > 0, "finished voice is handed back");
    }
}
