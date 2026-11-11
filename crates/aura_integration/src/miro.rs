//! Thin Aura host for Miro sessions backed by `imp_mir`.
//!
//! Mir fabric lives in `imp_mir`. This module maps Aura JSON `type` into record kinds
//! and pulls Imp graphs via `aura_imp` helpers.

use aura_imp::{graph_from_json_str, Graph};
use imp_mir::{KindFn, MiroError, MiroOpenOptions, MiroSession};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

/// Kind function: Aura JSON `type` when present, otherwise `"unknown"`.
pub fn aura_json_kind_fn() -> KindFn {
    Arc::new(|_path, content| aura_json_kind(content))
}

/// Extract Aura document `type` from JSON text.
pub fn aura_json_kind(content: &str) -> String {
    match serde_json::from_str::<Value>(content) {
        Ok(Value::Object(map)) => map
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        _ => "unknown".to_string(),
    }
}

#[derive(Debug, Error)]
pub enum AuraMiroError {
    #[error("{0}")]
    Miro(#[from] MiroError),
    #[error("pulled record `{id}` is not an Imp graph: {source}")]
    GraphParse {
        id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("no record `{id}` in Miro projection")]
    MissingRecord { id: String },
}

/// Open a Miro session over an Aura content root (text SoT → SQLite).
pub fn open_aura_miro_session(
    content_root: impl Into<PathBuf>,
    sqlite_path: impl Into<PathBuf>,
    start_watch: bool,
) -> Result<MiroSession, AuraMiroError> {
    Ok(MiroSession::open(
        MiroOpenOptions::new(content_root, sqlite_path)
            .with_kind_fn(aura_json_kind_fn())
            .with_start_watch(start_watch),
    )?)
}

/// Open an in-memory Miro projection for tests / ephemeral tools.
pub fn open_aura_miro_session_in_memory(
    content_root: impl Into<PathBuf>,
    start_watch: bool,
) -> Result<MiroSession, AuraMiroError> {
    Ok(MiroSession::open_in_memory(
        content_root,
        aura_json_kind_fn(),
        start_watch,
    )?)
}

/// Pull a record and parse it as an Imp `Graph` (Aura JSON may include a root `type`).
pub fn pull_graph(session: &MiroSession, id: &str) -> Result<Graph, AuraMiroError> {
    let record = session
        .pull_record(id)?
        .ok_or_else(|| AuraMiroError::MissingRecord { id: id.to_string() })?;
    graph_from_json_str(&record.content).map_err(|source| AuraMiroError::GraphParse {
        id: record.id,
        source,
    })
}

/// Sync a content root into `sqlite_path` and optionally pull one graph id for verification.
pub fn miro_sync_content_root(
    content_root: &Path,
    sqlite_path: &Path,
    pull_id: Option<&str>,
) -> Result<Option<Graph>, AuraMiroError> {
    let session = open_aura_miro_session(content_root, sqlite_path, false)?;
    match pull_id {
        Some(id) => Ok(Some(pull_graph(&session, id)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn aura_json_kind_reads_type() {
        assert_eq!(aura_json_kind(r#"{"type":"imp-graph"}"#), "imp-graph");
        assert_eq!(aura_json_kind("not json"), "unknown");
    }

    #[test]
    fn miro_sync_pulls_demo_shaped_graph() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("content");
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(
            root.join("demo-01.json"),
            r#"{"type":"imp-graph","nodes":{"n":{"id":"n","type":"time","inputs":{}}},"edges":{}}"#,
        )
        .expect("write");
        let db = dir.path().join("proj.db");

        let graph = miro_sync_content_root(&root, &db, Some("demo-01.json"))
            .expect("sync")
            .expect("pulled");
        assert!(graph.nodes.contains_key("n"));
    }
}
