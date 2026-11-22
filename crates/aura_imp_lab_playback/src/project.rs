//! Resolve records a graph references from the Imp Lab project's Tome data stores.
//!
//! Imp Lab passes the open project's config file (`imp-project.json`, a Tome v2 config) as
//! the neighbor path. A graph edge whose endpoint is not one of the graph's own nodes names
//! another record by node id: a notation (played as a score) or a node of another graph.
//! Lab stores each document as one Tome node with its body in a structured column
//! (`notation` / `graph`), so lookups read node files through `tome_flatfile`.

use aura_imp::{graph_from_json_str, Graph, NodeId};
use aura_integration::score_from_notation_str;
use aura_sequencing::ScoreSignal;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tome_db::{normalize_server_config, parse_server_config, ConfigEnv};
use tome_flatfile::FlatfileStore;

/// Record body columns written by Imp Lab (`lab_model.rs`).
const NOTATION_COLUMN: &str = "notation";
const GRAPH_COLUMN: &str = "graph";

/// Whether `path` is a Tome v2 project config (as opposed to a legacy graph file).
pub fn is_project_config(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .is_some_and(|v| v.get("version") == Some(&Value::from(2)) && v.get("dataStores").is_some())
}

/// Every flatfile store the project configures.
fn open_project_stores(config_path: &Path) -> Result<FlatfileStore, String> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;
    let raw: Value = serde_json::from_str(&text).map_err(|e| format!("project config: {e}"))?;
    let env = ConfigEnv {
        cwd: config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        ..Default::default()
    };
    let config = parse_server_config(&raw).map_err(|e| e.to_string())?;
    let config = normalize_server_config(&config, &env).map_err(|e| e.to_string())?;
    FlatfileStore::open(&config.corpora(&env)).map_err(|e| e.to_string())
}

/// What a referenced node id resolves to.
enum Found {
    Score(ScoreSignal),
    GraphNode(aura_imp::Node),
}

fn notation_score(notation: &Value) -> Result<ScoreSignal, String> {
    let mut notation = notation.clone();
    if let Some(obj) = notation.as_object_mut() {
        obj.entry("type").or_insert_with(|| Value::from("notation"));
    }
    score_from_notation_str(&notation.to_string()).map(ScoreSignal::finite)
}

/// Look up `wanted` ids: a notation record by its node id (or the `id` inside its body), or a
/// node inside another graph record.
fn find_records(
    store: &FlatfileStore,
    wanted: &BTreeSet<NodeId>,
) -> Result<BTreeMap<NodeId, Found>, String> {
    let mut found = BTreeMap::new();
    for record_id in store.list_node_ids().map_err(|e| e.to_string())? {
        if found.len() == wanted.len() {
            break;
        }
        let Some(node) = store.read_node(&record_id).map_err(|e| e.to_string())? else {
            continue;
        };
        if let Some(notation) = node.properties.get(NOTATION_COLUMN) {
            let inner_id = notation.get("id").and_then(Value::as_str);
            for id in [Some(record_id.as_str()), inner_id].into_iter().flatten() {
                if wanted.contains(id) && !found.contains_key(id) {
                    let score = notation_score(notation)
                        .map_err(|e| format!("notation record {record_id}: {e}"))?;
                    found.insert(id.to_string(), Found::Score(score));
                }
            }
        }
        if let Some(graph) = node.properties.get(GRAPH_COLUMN) {
            let has_wanted = graph
                .get("nodes")
                .and_then(Value::as_object)
                .is_some_and(|nodes| {
                    nodes
                        .keys()
                        .any(|k| wanted.contains(k) && !found.contains_key(k))
                });
            if !has_wanted {
                continue;
            }
            let other = graph_from_json_str(&graph.to_string())
                .map_err(|e| format!("graph record {record_id}: {e}"))?;
            for (id, graph_node) in other.nodes {
                if wanted.contains(&id) && !found.contains_key(&id) {
                    found.insert(id, Found::GraphNode(graph_node));
                }
            }
        }
    }
    Ok(found)
}

/// Pull every record `graph` references from the project at `config_path`. Remote graph
/// nodes are copied into `graph`; notations come back as score bindings for translation.
/// Ids that match no record are left for translation to report.
pub fn resolve_project_references(
    graph: &mut Graph,
    config_path: &Path,
) -> Result<BTreeMap<NodeId, ScoreSignal>, String> {
    let wanted: BTreeSet<NodeId> = graph
        .edges
        .values()
        .flat_map(|e| [e.from.node.clone(), e.to.node.clone()])
        .filter(|id| !graph.nodes.contains_key(id))
        .collect();
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    let store = open_project_stores(config_path)?;
    let mut bindings = BTreeMap::new();
    for (id, found) in find_records(&store, &wanted)? {
        match found {
            Found::Score(score) => {
                bindings.insert(id, score);
            }
            Found::GraphNode(node) => {
                graph.nodes.insert(id, node);
            }
        }
    }
    Ok(bindings)
}
