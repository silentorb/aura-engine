//! Thin Aura host for Miro sessions backed by `imp_mir`.
//!
//! Mir fabric lives in `imp_mir`. This module maps Aura JSON `type` into record kinds,
//! indexes tip NodeIds onto opaque Mir record ids, and pulls Imp graphs via `aura_imp`.

use aura_imp::{
    concrete_type, get_node_definitions, graph_from_json_str, Graph, Node, NodeDefinition, NodeId,
    Port, Registry,
};
use aura_imp::signals::SCORE;
use imp_mir::{KindFn, MiroError, MiroOpenOptions};
use imp_typecheck::{NodeResolver, ResolvedEndpoint};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

pub use imp_mir::{MiroSession, StoreRecord};

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

/// Host SQLite projection path for an Aura Miro content root.
///
/// `{content_root}/.aura/miro-projection.db`
pub fn aura_miro_db_path(content_root: impl AsRef<Path>) -> PathBuf {
    content_root
        .as_ref()
        .join(".aura")
        .join("miro-projection.db")
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

/// Resolved Mir blob for a product NodeId.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiroNodeRef {
    /// Opaque Mir record id (host text-store key).
    pub mir_record_id: String,
    /// Aura JSON tip kind (`imp-graph`, `notation`, …).
    pub kind: String,
}

/// Host index: Imp NodeId → Mir record (path-shaped blob key stays internal).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MiroNodeIndex {
    by_node: BTreeMap<String, MiroNodeRef>,
}

impl MiroNodeIndex {
    /// Empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lookup a NodeId.
    pub fn get(&self, node_id: &str) -> Option<&MiroNodeRef> {
        self.by_node.get(node_id)
    }

    /// Iterate NodeId → Mir ref entries in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &MiroNodeRef)> {
        self.by_node.iter().map(|(id, node_ref)| (id.as_str(), node_ref))
    }

    /// Number of indexed NodeIds.
    pub fn len(&self) -> usize {
        self.by_node.len()
    }

    /// Whether the index has no entries.
    pub fn is_empty(&self) -> bool {
        self.by_node.is_empty()
    }

    fn insert(&mut self, node_id: impl Into<String>, mir_record_id: &str, kind: &str) {
        self.by_node.insert(
            node_id.into(),
            MiroNodeRef {
                mir_record_id: mir_record_id.to_string(),
                kind: kind.to_string(),
            },
        );
    }
}

/// Synthetic catalog id for host-resolved notation tips (not an Imp product type).
pub const NOTATION_TIP_NODE_TYPE: &str = "#miro/notation";

/// Port templates for a notation tip NodeId (single `score` output).
pub fn notation_tip_definition() -> NodeDefinition {
    NodeDefinition {
        id: NOTATION_TIP_NODE_TYPE.into(),
        name: None,
        type_params: Vec::new(),
        implementation: None,
        inputs: Default::default(),
        outputs: BTreeMap::from([(
            "score".into(),
            Port {
                id: "score".into(),
                signal_type: concrete_type(SCORE, vec![]),
                default_value: None,
                variadic: false,
            },
        )]),
        body: None,
        bindings: None,
    }
}

/// Synthetic Imp view of a notation tip for typecheck host resolve.
pub fn notation_tip_endpoint(node_id: &str) -> ResolvedEndpoint {
    ResolvedEndpoint {
        node: Node {
            id: node_id.to_string(),
            title: None,
            node_type: NOTATION_TIP_NODE_TYPE.to_string(),
            type_args: Vec::new(),
            inputs: Default::default(),
        },
        definition: notation_tip_definition(),
    }
}

/// A NodeId resolved from Miro into a fragment suitable for `Graph.nodes` insert.
///
/// Only `imp-graph` tips produce a local node copy. Notation tips are host-resolved
/// remotes (see [`MiroContentResolver`]), not Imp stubs.
#[derive(Debug, Clone, PartialEq)]
pub struct MiroLocalNode {
    /// Local instance map entry (same NodeId the edge already names).
    pub node: Node,
    /// Opaque Mir blob key for the tip that contained this node.
    pub mir_record_id: String,
    /// Tip kind (`imp-graph`, …).
    pub kind: String,
}

/// Content-root Miro index as an Imp [`NodeResolver`] for typecheck / translate.
pub struct MiroContentResolver<'a> {
    /// Authoring content root.
    pub content_root: &'a Path,
    /// NodeId → Mir record index.
    pub index: &'a MiroNodeIndex,
    /// Registry for looking up remote `imp-graph` node definitions.
    pub registry: &'a Registry,
}

impl NodeResolver for MiroContentResolver<'_> {
    fn resolve_endpoint(&self, node_id: &NodeId) -> Option<ResolvedEndpoint> {
        let node_ref = self.index.get(node_id)?;
        match node_ref.kind.as_str() {
            "notation" => Some(notation_tip_endpoint(node_id)),
            "imp-graph" => {
                let path = authoring_path_for_mir_record(self.content_root, &node_ref.mir_record_id);
                let content = fs::read_to_string(path).ok()?;
                let graph = graph_from_json_str(&content).ok()?;
                let node = graph.nodes.get(node_id)?.clone();
                let definition = get_node_definitions(self.registry, &node.node_type)
                    .first()
                    .cloned()?;
                Some(ResolvedEndpoint { node, definition })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum AuraMiroError {
    #[error("{0}")]
    Miro(#[from] MiroError),
    #[error("failed to prepare Miro projection directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("pulled record `{id}` is not an Imp graph: {source}")]
    GraphParse {
        id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("no Mir record `{id}` in Miro projection")]
    MissingRecord { id: String },
    #[error("no Miro record indexed for node `{node_id}`")]
    MissingNode { node_id: String },
    #[error("indexed tip `{mir_record_id}` kind `{kind}` cannot resolve a local graph node")]
    UnsupportedResolveKind {
        /// Opaque Mir blob key.
        mir_record_id: String,
        /// Tip kind from the index.
        kind: String,
    },
    #[error("node `{node_id}` missing from pulled imp-graph tip `{mir_record_id}`")]
    NodeAbsentInTip {
        /// Product NodeId.
        node_id: String,
        /// Opaque Mir blob key.
        mir_record_id: String,
    },
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

/// Open a Miro session using [`aura_miro_db_path`] under `content_root`.
///
/// Creates the `.aura` directory when needed.
pub fn open_aura_miro_session_for_root(
    content_root: impl Into<PathBuf>,
    start_watch: bool,
) -> Result<MiroSession, AuraMiroError> {
    let content_root = content_root.into();
    let sqlite_path = aura_miro_db_path(&content_root);
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    open_aura_miro_session(content_root, sqlite_path, start_watch)
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

/// Build a NodeId → Mir-record index by scanning the content-root text SoT.
///
/// - `imp-graph` tips contribute every local `nodes.*.id`.
/// - `notation` tips contribute their tip-level `id` when present.
///
/// Dot-directories (including `.aura`) are skipped, matching the text-store walk.
pub fn build_miro_node_index(content_root: impl AsRef<Path>) -> Result<MiroNodeIndex, AuraMiroError> {
    let root = content_root.as_ref();
    let mut index = MiroNodeIndex::new();
    walk_index(root, root, &mut index)?;
    Ok(index)
}

/// Resolve a product NodeId through a host index.
pub fn resolve_node<'a>(
    index: &'a MiroNodeIndex,
    node_id: &str,
) -> Result<&'a MiroNodeRef, AuraMiroError> {
    index
        .get(node_id)
        .ok_or_else(|| AuraMiroError::MissingNode {
            node_id: node_id.to_string(),
        })
}

/// Pull a Mir blob by product NodeId (index → opaque Mir `pull_record`).
pub fn pull_by_node_id(
    session: &MiroSession,
    index: &MiroNodeIndex,
    node_id: &str,
) -> Result<StoreRecord, AuraMiroError> {
    let node_ref = resolve_node(index, node_id)?;
    session
        .pull_record(&node_ref.mir_record_id)?
        .ok_or_else(|| AuraMiroError::MissingRecord {
            id: node_ref.mir_record_id.clone(),
        })
}

/// Resolve a product NodeId into a local `Graph.nodes` fragment.
///
/// Only `imp-graph` tips copy a real Imp node. Notation tips are not Imp nodes —
/// open the tip document / host-resolve them instead of inserting a proxy.
pub fn resolve_local_node(
    session: &MiroSession,
    index: &MiroNodeIndex,
    node_id: &str,
) -> Result<MiroLocalNode, AuraMiroError> {
    let node_ref = resolve_node(index, node_id)?;
    let record = session
        .pull_record(&node_ref.mir_record_id)?
        .ok_or_else(|| AuraMiroError::MissingRecord {
            id: node_ref.mir_record_id.clone(),
        })?;
    match node_ref.kind.as_str() {
        "imp-graph" => {
            let graph =
                graph_from_json_str(&record.content).map_err(|source| AuraMiroError::GraphParse {
                    id: record.id.clone(),
                    source,
                })?;
            let node = graph
                .nodes
                .get(node_id)
                .cloned()
                .ok_or_else(|| AuraMiroError::NodeAbsentInTip {
                    node_id: node_id.to_string(),
                    mir_record_id: node_ref.mir_record_id.clone(),
                })?;
            Ok(MiroLocalNode {
                node,
                mir_record_id: node_ref.mir_record_id.clone(),
                kind: node_ref.kind.clone(),
            })
        }
        other => Err(AuraMiroError::UnsupportedResolveKind {
            mir_record_id: node_ref.mir_record_id.clone(),
            kind: other.to_string(),
        }),
    }
}

/// Pull a Mir blob by opaque Mir record id and parse it as an Imp `Graph`.
///
/// `mir_record_id` is a Mir store key (Aura text store currently uses a
/// root-relative path). Prefer [`pull_by_node_id`] for product NodeId lookups.
pub fn pull_graph(session: &MiroSession, mir_record_id: &str) -> Result<Graph, AuraMiroError> {
    let record = session
        .pull_record(mir_record_id)?
        .ok_or_else(|| AuraMiroError::MissingRecord {
            id: mir_record_id.to_string(),
        })?;
    graph_from_json_str(&record.content).map_err(|source| AuraMiroError::GraphParse {
        id: record.id,
        source,
    })
}

/// Sync a content root into `sqlite_path` and optionally pull one graph by Mir record id.
pub fn miro_sync_content_root(
    content_root: &Path,
    sqlite_path: &Path,
    pull_mir_record_id: Option<&str>,
) -> Result<Option<Graph>, AuraMiroError> {
    let session = open_aura_miro_session(content_root, sqlite_path, false)?;
    match pull_mir_record_id {
        Some(id) => Ok(Some(pull_graph(&session, id)?)),
        None => Ok(None),
    }
}

fn walk_index(root: &Path, dir: &Path, index: &mut MiroNodeIndex) -> Result<(), AuraMiroError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk_index(root, &path, index)?;
        } else if path.is_file() {
            index_file(root, &path, index)?;
        }
    }
    Ok(())
}

fn index_file(root: &Path, path: &Path, index: &mut MiroNodeIndex) -> Result<(), AuraMiroError> {
    let content = fs::read_to_string(path)?;
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return Ok(());
    };
    let Some(mir_record_id) = relativize(root, path) else {
        return Ok(());
    };
    index_tip_value(&mir_record_id, &value, index);
    Ok(())
}

fn index_tip_value(mir_record_id: &str, value: &Value, index: &mut MiroNodeIndex) {
    let Some(tip_type) = value.get("type").and_then(|v| v.as_str()) else {
        return;
    };
    match tip_type {
        "imp-graph" => {
            if let Some(nodes) = value.get("nodes").and_then(|n| n.as_object()) {
                for node_id in nodes.keys() {
                    index.insert(node_id.clone(), mir_record_id, tip_type);
                }
            }
        }
        "notation" => {
            if let Some(node_id) = value.get("id").and_then(|v| v.as_str()) {
                if !node_id.is_empty() {
                    index.insert(node_id, mir_record_id, tip_type);
                }
            }
        }
        _ => {}
    }
}

fn relativize(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
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
    fn aura_miro_db_path_is_under_dot_aura() {
        let path = aura_miro_db_path("/content/root");
        assert_eq!(
            path,
            PathBuf::from("/content/root/.aura/miro-projection.db")
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
    fn miro_sync_pulls_demo_shaped_graph_by_mir_record_id() {
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
        let graph = miro_sync_content_root(&root, &db, Some("demo-01.json"))
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

        let session = open_aura_miro_session_for_root(&root, false).expect("open");
        assert!(aura_miro_db_path(&root).is_file());
        let graph = pull_graph(&session, "demo-01.json").expect("pull");
        assert!(graph.nodes.contains_key("n"));
    }

    #[test]
    fn node_index_maps_graph_and_notation_ids() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("content");
        fs::create_dir_all(root.join("demos")).expect("mkdir");
        fs::write(
            root.join("g.json"),
            r#"{"type":"imp-graph","nodes":{"NODEA":{"id":"NODEA","type":"time","inputs":{}},"NODEB":{"id":"NODEB","type":"output","inputs":{}}},"edges":{}}"#,
        )
        .expect("write");
        fs::write(
            root.join("demos/score.json"),
            r#"{"type":"notation","id":"TIPID00000000000000000001","version":1,"tempoBpm":120.0,"timeSignature":{"beatsPerBar":4,"beatUnit":4},"events":[]}"#,
        )
        .expect("write");
        // Hidden / projection dirs must not be indexed.
        fs::create_dir_all(root.join(".aura")).expect("mkdir .aura");
        fs::write(
            root.join(".aura/ignored.json"),
            r#"{"type":"imp-graph","nodes":{"HIDDEN":{"id":"HIDDEN","type":"time","inputs":{}}},"edges":{}}"#,
        )
        .expect("write hidden");

        let index = build_miro_node_index(&root).expect("index");
        assert_eq!(
            index.get("NODEA"),
            Some(&MiroNodeRef {
                mir_record_id: "g.json".into(),
                kind: "imp-graph".into(),
            })
        );
        assert_eq!(
            index.get("NODEB").map(|r| r.mir_record_id.as_str()),
            Some("g.json")
        );
        assert_eq!(
            index.get("TIPID00000000000000000001"),
            Some(&MiroNodeRef {
                mir_record_id: "demos/score.json".into(),
                kind: "notation".into(),
            })
        );
        assert!(index.get("HIDDEN").is_none());
    }

    #[test]
    fn pull_by_node_id_uses_index() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("content");
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(
            root.join("g.json"),
            r#"{"type":"imp-graph","nodes":{"NODEA":{"id":"NODEA","type":"time","inputs":{}}},"edges":{}}"#,
        )
        .expect("write");

        let session = open_aura_miro_session_in_memory(&root, false).expect("session");
        let index = build_miro_node_index(&root).expect("index");
        let record = pull_by_node_id(&session, &index, "NODEA").expect("pull");
        assert_eq!(record.id, "g.json");
        assert_eq!(record.kind, "imp-graph");
        assert!(pull_by_node_id(&session, &index, "missing").is_err());
    }

    #[test]
    fn index_iter_lists_node_ids() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("content");
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(
            root.join("g.json"),
            r#"{"type":"imp-graph","nodes":{"NODEA":{"id":"NODEA","type":"time","inputs":{}},"NODEB":{"id":"NODEB","type":"output","inputs":{}}},"edges":{}}"#,
        )
        .expect("write");
        let index = build_miro_node_index(&root).expect("index");
        let ids: Vec<_> = index.iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["NODEA", "NODEB"]);
    }

    #[test]
    fn resolve_local_node_copies_imp_graph_node_not_notation() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("content");
        fs::create_dir_all(root.join("demos")).expect("mkdir");
        fs::write(
            root.join("g.json"),
            r#"{"type":"imp-graph","nodes":{"NODEA":{"id":"NODEA","type":"time","title":"Clock","inputs":{}}},"edges":{}}"#,
        )
        .expect("write");
        fs::write(
            root.join("demos/score.json"),
            r#"{"type":"notation","id":"TIPID00000000000000000001","version":1,"tempoBpm":120.0,"timeSignature":{"beatsPerBar":4,"beatUnit":4},"events":[]}"#,
        )
        .expect("write");

        let session = open_aura_miro_session_in_memory(&root, false).expect("session");
        let index = build_miro_node_index(&root).expect("index");

        let graph_frag = resolve_local_node(&session, &index, "NODEA").expect("graph node");
        assert_eq!(graph_frag.kind, "imp-graph");
        assert_eq!(graph_frag.mir_record_id, "g.json");
        assert_eq!(graph_frag.node.id, "NODEA");
        assert_eq!(graph_frag.node.node_type, "time");
        assert_eq!(graph_frag.node.title.as_deref(), Some("Clock"));

        let err = resolve_local_node(&session, &index, "TIPID00000000000000000001")
            .expect_err("notation is not a local Imp node");
        assert!(matches!(
            err,
            AuraMiroError::UnsupportedResolveKind { kind, .. } if kind == "notation"
        ));
    }
}
