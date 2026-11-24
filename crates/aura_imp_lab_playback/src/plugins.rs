//! Plugins in Imp Lab: the installed-plugin lists a plugin node's `plugin` literal chooses
//! from, and plugin editors opened from a node, whose edits flow back into the node.
//!
//! - Topics `aura.plugins.instruments` / `aura.plugins.effects` hold `[{value, label}]` of the
//!   installed CLAP plugins, published once the scan (off the startup path) finishes.
//! - The `node_action` request with action `plugin.open_gui` opens the node's plugin GUI
//!   configured from the node's data (or focuses the one already open for that node).
//! - While an editor is open, each state the plugin saves is published on `aura.plugins.edits`
//!   as a node edit: `state` set to the new blob and `params` removed (the state holds every
//!   parameter). One editor is one edit session, so Lab keeps it as one undo step.

use crate::{plugin_host, publish, PlaybackServiceError};
use aura_clap::{EditorEvent, EditorHandle, PluginInfo};
use aura_imp::plugin::{PLUGIN_PARAMS_PORT, PLUGIN_STATE_PORT};
use aura_imp::Node;
use aura_integration::plugin_node_settings;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub const TOPIC_INSTRUMENTS: &str = "aura.plugins.instruments";
pub const TOPIC_EFFECTS: &str = "aura.plugins.effects";
pub const TOPIC_EDITS: &str = "aura.plugins.edits";
pub const ACTION_OPEN_GUI: &str = "plugin.open_gui";

/// CLAP feature marking an instrument (synthesizer, sampler, …).
const FEATURE_INSTRUMENT: &str = "instrument";

fn option(plugin: &PluginInfo) -> Value {
    let label = if plugin.vendor.is_empty() {
        plugin.name.clone()
    } else {
        format!("{} ({})", plugin.name, plugin.vendor)
    };
    json!({ "value": plugin.id, "label": label })
}

/// The plugin lists for each node type: instruments, and everything else as effects.
pub fn plugin_options(plugins: &[PluginInfo]) -> (Vec<Value>, Vec<Value>) {
    let mut sorted: Vec<&PluginInfo> = plugins.iter().collect();
    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    let (instruments, effects): (Vec<&PluginInfo>, Vec<&PluginInfo>) = sorted
        .into_iter()
        .partition(|p| p.features.iter().any(|f| f == FEATURE_INSTRUMENT));
    (
        instruments.into_iter().map(option).collect(),
        effects.into_iter().map(option).collect(),
    )
}

/// Scans for installed plugins on a background thread and publishes the lists.
pub(crate) fn publish_plugin_lists() {
    let spawned = std::thread::Builder::new()
        .name("aura-plugin-scan".into())
        .spawn(|| {
            let discovery = plugin_host().discover();
            for failure in &discovery.failures {
                eprintln!(
                    "aura: skipped plugin bundle {}: {}",
                    failure.bundle.display(),
                    failure.reason
                );
            }
            let (instruments, effects) = plugin_options(&discovery.plugins);
            publish(TOPIC_INSTRUMENTS, &instruments);
            publish(TOPIC_EFFECTS, &effects);
        });
    if let Err(error) = spawned {
        eprintln!("aura: could not start the plugin scan: {error}");
    }
}

/// Open editors, by node id.
fn editors() -> &'static Mutex<HashMap<String, EditorHandle>> {
    static EDITORS: OnceLock<Mutex<HashMap<String, EditorHandle>>> = OnceLock::new();
    EDITORS.get_or_init(Default::default)
}

static EDIT_SEQ: AtomicU64 = AtomicU64::new(0);
static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

/// The node edit published for a saved plugin state.
pub fn state_edit(seq: u64, session: &str, node: &str, state: &[u8]) -> Value {
    json!({
        "seq": seq,
        "session": session,
        "node": node,
        "inputs": {
            PLUGIN_STATE_PORT: base64::engine::general_purpose::STANDARD.encode(state),
            PLUGIN_PARAMS_PORT: null,
        },
    })
}

/// Answers a `node_action` request.
pub fn node_action(value: &Value) -> Result<Value, PlaybackServiceError> {
    let bad = |message: String| PlaybackServiceError::Request(message);
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("node_action needs a string `action`".into()))?;
    if action != ACTION_OPEN_GUI {
        return Err(bad(format!("unknown node action `{action}`")));
    }
    let node: Node = serde_json::from_value(
        value
            .get("node")
            .cloned()
            .ok_or_else(|| bad("node_action needs `node`".into()))?,
    )
    .map_err(|e| bad(format!("invalid node: {e}")))?;
    open_gui(&node)?;
    Ok(json!({}))
}

fn open_gui(node: &Node) -> Result<(), PlaybackServiceError> {
    let mut open = editors().lock().map_err(|_| PlaybackServiceError::Poisoned)?;
    if let Some(editor) = open.get(&node.id) {
        if editor.is_open() {
            editor.focus();
            return Ok(());
        }
        open.remove(&node.id);
    }
    let (plugin, settings) =
        plugin_node_settings(node).map_err(|e| PlaybackServiceError::Translate(e.to_string()))?;
    let session = format!(
        "{}:{}",
        node.id,
        SESSION_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let title = node.title.clone().unwrap_or_else(|| plugin.clone());
    let node_id = node.id.clone();
    let editor = plugin_host()
        .open_editor(&plugin, &settings, &title, move |event| match event {
            EditorEvent::State(state) => {
                let seq = EDIT_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
                publish(TOPIC_EDITS, &state_edit(seq, &session, &node_id, &state));
            }
            EditorEvent::Closed => {
                // Runs on the CLAP main thread; a request holding the lock never waits on it.
                if let Ok(mut open) = editors().try_lock() {
                    open.remove(&node_id);
                }
            }
        })
        .map_err(|e| PlaybackServiceError::Playback(e.to_string()))?;
    open.insert(node.id.clone(), editor);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn info(id: &str, name: &str, features: &[&str]) -> PluginInfo {
        PluginInfo {
            id: id.into(),
            name: name.into(),
            vendor: "V".into(),
            version: "1".into(),
            features: features.iter().map(|f| f.to_string()).collect(),
            bundle: PathBuf::from("/x.clap"),
        }
    }

    #[test]
    fn plugins_split_into_instruments_and_effects_by_name() {
        let (instruments, effects) = plugin_options(&[
            info("b.synth", "beta", &["instrument", "synthesizer"]),
            info("a.verb", "Reverb", &["audio-effect"]),
            info("a.synth", "Alpha", &["instrument"]),
        ]);
        assert_eq!(
            instruments,
            vec![
                json!({"value": "a.synth", "label": "Alpha (V)"}),
                json!({"value": "b.synth", "label": "beta (V)"}),
            ]
        );
        assert_eq!(effects, vec![json!({"value": "a.verb", "label": "Reverb (V)"})]);
    }

    #[test]
    fn state_edits_replace_state_and_drop_params() {
        assert_eq!(
            state_edit(4, "n1:0", "n1", &[0, 0, 128, 63]),
            json!({
                "seq": 4,
                "session": "n1:0",
                "node": "n1",
                "inputs": { "state": "AACAPw==", "params": null },
            })
        );
    }

    #[test]
    fn the_pack_declares_plugin_choices_and_gui_actions() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lab-pack/extensions.json");
        let json = std::fs::read_to_string(path).expect("pack manifest");
        let config = imp_lab_extensions::parse_extensions_json(&json).expect("parses");
        let ui = imp_lab_extensions::node_ui(&config);
        assert!(ui.warnings.is_empty(), "{:?}", ui.warnings);
        let topics: Vec<(&str, &str)> = ui
            .literal_options
            .iter()
            .map(|o| (o.node_type.as_str(), o.options_topic.as_str()))
            .collect();
        assert_eq!(
            topics,
            vec![
                ("plugin_instrument", TOPIC_INSTRUMENTS),
                ("plugin_effect", TOPIC_EFFECTS)
            ]
        );
        assert_eq!(ui.actions.len(), 2);
        for action in &ui.actions {
            assert_eq!(action.action, ACTION_OPEN_GUI);
            assert_eq!(action.edits_topic.as_deref(), Some(TOPIC_EDITS));
            assert_eq!(action.service_id, crate::SERVICE_IMPLEMENTATION_ID);
        }
    }

    #[test]
    fn node_actions_reject_unknown_actions_and_nodes() {
        assert!(node_action(&json!({"type": "node_action", "action": "nope"})).is_err());
        assert!(node_action(&json!({"type": "node_action", "action": ACTION_OPEN_GUI})).is_err());
        let no_plugin = json!({
            "type": "node_action",
            "action": ACTION_OPEN_GUI,
            "node": {"id": "n1", "type": "plugin_instrument", "inputs": {}},
        });
        let error = node_action(&no_plugin).expect_err("no plugin id");
        assert!(error.to_string().contains("names no plugin"), "{error}");
    }
}
