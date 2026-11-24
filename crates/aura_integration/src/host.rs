//! Thin Aura host for Miro-profile sessions backed by `imp_mir`.
//!
//! Mir fabric lives in `imp_mir`. This module maps Aura JSON `type` into record kinds and pulls
//! Imp graphs via `aura_imp`. Resolving NodeIds that a graph references but does not hold is not
//! a host concern here: it belongs to Imp graph sources and Tome's node provider (`tome_imp`).

use aura_imp::{graph_from_json_str, Graph};
use imp_mir::{KindFn, SessionError, OpenOptions};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

pub use imp_mir::{Session, StoreRecord};

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

/// Host SQLite projection path for an Aura content root (Miro profile).
///
/// `{content_root}/.aura/projection.db`
pub fn aura_projection_db_path(content_root: impl AsRef<Path>) -> PathBuf {
    content_root
        .as_ref()
        .join(".aura")
        .join("projection.db")
}

/// Joins an opaque Mir record id onto a content root.
///
/// Aura's text store currently encodes Mir ids as root-relative paths with `/`
/// separators. Callers must pass a Mir record id from the host index — never a
/// product NodeId as if it were a path.
pub fn authoring_path_for_mir_record(content_root: &Path, mir_record_id: &str) -> PathBuf {
    let mut path = content_root.to_path_buf();
    for segment in mir_record_id.split('/').filter(|s| !s.is_empty()) {
        path.push(segment);
    }
    path
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("{0}")]
    Session(#[from] SessionError),
    #[error("failed to prepare projection directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("pulled record `{id}` is not an Imp graph: {source}")]
    GraphParse {
        id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("no Mir record `{id}` in projection")]
    MissingRecord { id: String },
}

/// Open a Miro-profile session over an Aura content root (text SoT → SQLite).
pub fn open_aura_session(
    content_root: impl Into<PathBuf>,
    sqlite_path: impl Into<PathBuf>,
    start_watch: bool,
) -> Result<Session, HostError> {
    Ok(Session::open(
        OpenOptions::new(content_root, sqlite_path)
            .with_kind_fn(aura_json_kind_fn())
            .with_start_watch(start_watch),
    )?)
}

/// Open a session using [`aura_projection_db_path`] under `content_root`.
///
/// Creates the `.aura` directory when needed.
pub fn open_aura_session_for_root(
    content_root: impl Into<PathBuf>,
    start_watch: bool,
) -> Result<Session, HostError> {
    let content_root = content_root.into();
    let sqlite_path = aura_projection_db_path(&content_root);
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    open_aura_session(content_root, sqlite_path, start_watch)
}

/// Open an in-memory projection for tests / ephemeral tools.
pub fn open_aura_session_in_memory(
    content_root: impl Into<PathBuf>,
    start_watch: bool,
) -> Result<Session, HostError> {
    Ok(Session::open_in_memory(
        content_root,
        aura_json_kind_fn(),
        start_watch,
    )?)
}

/// Pull a Mir blob by opaque Mir record id and parse it as an Imp `Graph`.
///
/// `mir_record_id` is a Mir store key (Aura text store currently uses a
/// root-relative path).
pub fn pull_graph(session: &Session, mir_record_id: &str) -> Result<Graph, HostError> {
    let record = session
        .pull_record(mir_record_id)?
        .ok_or_else(|| HostError::MissingRecord {
            id: mir_record_id.to_string(),
        })?;
    graph_from_json_str(&record.content).map_err(|source| HostError::GraphParse {
        id: record.id,
        source,
    })
}

/// Sync a content root into `sqlite_path` and optionally pull one graph by Mir record id.
pub fn sync_content_root(
    content_root: &Path,
    sqlite_path: &Path,
    pull_mir_record_id: Option<&str>,
) -> Result<Option<Graph>, HostError> {
    let session = open_aura_session(content_root, sqlite_path, false)?;
    match pull_mir_record_id {
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
    fn aura_projection_db_path_is_under_dot_aura() {
        let path = aura_projection_db_path("/content/root");
        assert_eq!(
            path,
            PathBuf::from("/content/root/.aura/projection.db")
        );
    }

    #[test]
    fn authoring_path_splits_mir_record_segments() {
        assert_eq!(
            authoring_path_for_mir_record(Path::new("/root"), "demos/a.json"),
            PathBuf::from("/root/demos/a.json")
        );
    }

    #[test]
    fn sync_pulls_demo_shaped_graph_by_mir_record_id() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("content");
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(
            root.join("demo-01.json"),
            r#"{"type":"imp-graph","nodes":{"n":{"id":"n","type":"time","inputs":{}}},"edges":{}}"#,
        )
        .expect("write");
        let db = dir.path().join("proj.db");

        // "demo-01.json" is the Mir blob key (text-store encoding), not a NodeId.
        let graph = sync_content_root(&root, &db, Some("demo-01.json"))
            .expect("sync")
            .expect("pulled");
        assert!(graph.nodes.contains_key("n"));
    }

    #[test]
    fn open_for_root_creates_projection_and_pulls_by_mir_record_id() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("content");
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(
            root.join("demo-01.json"),
            r#"{"type":"imp-graph","nodes":{"n":{"id":"n","type":"time","inputs":{}}},"edges":{}}"#,
        )
        .expect("write");

        let session = open_aura_session_for_root(&root, false).expect("open");
        assert!(aura_projection_db_path(&root).is_file());
        let graph = pull_graph(&session, "demo-01.json").expect("pull");
        assert!(graph.nodes.contains_key("n"));
    }
}
