//! Load Aura JSON `notation` documents into engine [`Score`] values.

use aura_sequencing::{NoteEvent, Score, Semitone, Tempo, TimeSignature};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Errors loading or mapping an Aura notation JSON file.
#[derive(Debug, Error)]
pub enum NotationFileError {
    #[error("failed to read notation file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("notation file {path}: {message}")]
    Invalid { path: String, message: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotationDocument {
    #[serde(rename = "type")]
    type_id: String,
    version: u32,
    tempo_bpm: f64,
    time_signature: NotationTimeSignatureDoc,
    #[serde(default)]
    events: Vec<NotationEventDoc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotationTimeSignatureDoc {
    beats_per_bar: u8,
    beat_unit: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotationEventDoc {
    semitone: i16,
    start_beats: f64,
    duration_beats: f64,
    #[serde(default = "default_velocity")]
    velocity: f32,
}

fn default_velocity() -> f32 {
    1.0
}

/// Loads an Aura `notation` JSON file and maps it to a [`Score`].
pub fn score_from_notation_path(path: &Path) -> Result<Score, NotationFileError> {
    let path_str = path.to_string_lossy().into_owned();
    let contents = fs::read_to_string(path).map_err(|source| NotationFileError::Read {
        path: path_str.clone(),
        source,
    })?;
    score_from_notation_str(&contents).map_err(|message| NotationFileError::Invalid {
        path: path_str,
        message,
    })
}

/// Parses an Aura `notation` JSON string and maps it to a [`Score`].
pub fn score_from_notation_str(json: &str) -> Result<Score, String> {
    let doc: NotationDocument =
        serde_json::from_str(json).map_err(|err| format!("JSON parse error: {err}"))?;
    if doc.type_id != "notation" {
        return Err(format!(
            "expected type \"notation\", got \"{}\"",
            doc.type_id
        ));
    }
    if doc.version != 1 {
        return Err(format!("unsupported notation version {}", doc.version));
    }

    let tempo = Tempo::new(doc.tempo_bpm).map_err(|err| err.to_string())?;
    let time_signature = TimeSignature {
        beats_per_bar: doc.time_signature.beats_per_bar,
        beat_unit: doc.time_signature.beat_unit,
    };
    let events = doc
        .events
        .into_iter()
        .map(|event| {
            NoteEvent::new(
                Semitone(event.semitone),
                event.start_beats,
                event.duration_beats,
            )
            .with_velocity(event.velocity)
        })
        .collect();

    Ok(Score::new(time_signature, tempo, events))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_from_notation_str_maps_events() {
        let json = r#"{
            "type": "notation",
            "version": 1,
            "tempoBpm": 120.0,
            "timeSignature": { "beatsPerBar": 4, "beatUnit": 4 },
            "events": [
                { "semitone": 57, "startBeats": 0.0, "durationBeats": 1.0, "velocity": 1.0 },
                { "semitone": 60, "startBeats": 1.0, "durationBeats": 1.0 }
            ]
        }"#;

        let score = score_from_notation_str(json).expect("score");
        assert_eq!(score.tempo.bpm, 120.0);
        assert_eq!(score.events.len(), 2);
        assert_eq!(score.events[0].semitone, Semitone(57));
        assert_eq!(score.events[1].velocity, 1.0);
    }

    #[test]
    fn score_from_notation_str_rejects_wrong_type() {
        let json = r#"{
            "type": "imp-graph",
            "version": 1,
            "tempoBpm": 120.0,
            "timeSignature": { "beatsPerBar": 4, "beatUnit": 4 },
            "events": []
        }"#;
        let err = score_from_notation_str(json).expect_err("type");
        assert!(err.contains("notation"));
    }
}
