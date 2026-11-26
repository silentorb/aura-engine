//! Plugin nodes (`plugin_instrument`, `plugin_effect`): third-party plugins resolved by the
//! host's [`PluginResolver`](aura_plugin_host::PluginResolver).
//!
//! Both modes build a [`RenderCache`]: in deterministic mode it renders the plugin forward
//! from the fully known score and answers any range from what it kept; in a streaming voice
//! instance it plays the voice's live note.
//! Streaming programs whose output is a chain of plugins instead share one instance per
//! plugin across every voice (see `StreamingProgram::open`).

use super::streaming::NodeCtx;
use super::{NodeValue, TranslateError};
use aura_imp::plugin::{
    PLUGIN_ID_PORT, PLUGIN_INSTRUMENT, PLUGIN_PARAMS_PORT, PLUGIN_STATE_PORT,
};
use aura_imp::{Node, PrimitiveValue};
use aura_plugin_host::{
    EventSource, LayoutStage, NoteEventSource, ParamSetting, PluginSettings, ProcessorConfig,
    ProcessorFactory, RenderCache, RenderMode, BLOCK_FRAMES,
};
use aura_render::{AudioSignal, Layout};
use aura_sample::SampleRate;
use base64::Engine;
use std::sync::Arc;

fn literal(node: &Node, port: &str) -> Result<String, TranslateError> {
    match node.inputs.get(port) {
        None => Ok(String::new()),
        Some(PrimitiveValue::String(value)) => Ok(value.clone()),
        Some(_) => Err(TranslateError::ExpectedString {
            node: node.id.clone(),
            port: port.into(),
        }),
    }
}

/// The plugin id and settings (decoded state, parameter values) a plugin node embeds.
fn plugin_ref(node: &Node) -> Result<(String, PluginSettings), TranslateError> {
    let id = literal(node, PLUGIN_ID_PORT)?;
    if id.trim().is_empty() {
        return Err(TranslateError::MissingPluginId {
            node: node.id.clone(),
        });
    }
    let state = literal(node, PLUGIN_STATE_PORT)?;
    let state = if state.is_empty() {
        None
    } else {
        Some(
            base64::engine::general_purpose::STANDARD
                .decode(state.trim())
                .map_err(|_| TranslateError::InvalidPluginState {
                    node: node.id.clone(),
                })?,
        )
    };
    let params = parse_params(node, &literal(node, PLUGIN_PARAMS_PORT)?)?;
    Ok((id, PluginSettings { state, params }))
}

/// The plugin id and settings a plugin node (`plugin_instrument` / `plugin_effect`) embeds, as
/// translation reads them. Hosts use this to open a node's plugin outside translation (an
/// editor).
pub fn plugin_node_settings(node: &Node) -> Result<(String, PluginSettings), TranslateError> {
    plugin_ref(node)
}

/// Parses a `params` literal: a JSON object of CLAP parameter id (decimal) to plain value.
fn parse_params(node: &Node, text: &str) -> Result<Vec<ParamSetting>, TranslateError> {
    let invalid = |reason: String| TranslateError::InvalidPluginParams {
        node: node.id.clone(),
        reason,
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| invalid(e.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("expected a JSON object of parameter id to value".into()))?;
    object
        .iter()
        .map(|(key, value)| {
            let id = key
                .trim()
                .parse::<u32>()
                .map_err(|_| invalid(format!("`{key}` is not a parameter id")))?;
            let value = value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| invalid(format!("parameter {id} needs a finite number")))?;
            Ok(ParamSetting { id, value })
        })
        .collect()
}

/// Resolves a plugin node's factory through the translation's plugin host.
pub(super) fn plugin_factory(
    node: &Node,
    plugins: Option<&dyn aura_plugin_host::PluginResolver>,
) -> Result<Arc<dyn ProcessorFactory>, TranslateError> {
    let (id, settings) = plugin_ref(node)?;
    let Some(plugins) = plugins else {
        return Err(TranslateError::NoPluginHost {
            node: node.id.clone(),
            plugin: id,
        });
    };
    plugins
        .resolve(&id, &settings)
        .map_err(|source| TranslateError::Plugin {
            node: node.id.clone(),
            source,
        })
}

/// An activated instance of a plugin node's plugin, adapted to `layout`.
pub(super) fn plugin_stage(
    node: &Node,
    factory: &dyn ProcessorFactory,
    sample_rate: SampleRate,
    mode: RenderMode,
    layout: Layout,
) -> Result<LayoutStage, TranslateError> {
    let processor = factory
        .create(ProcessorConfig {
            sample_rate,
            max_block_frames: BLOCK_FRAMES,
            mode,
        })
        .map_err(|source| TranslateError::Plugin {
            node: node.id.clone(),
            source,
        })?;
    Ok(LayoutStage::new(processor, BLOCK_FRAMES as usize, layout))
}

/// Translates a plugin node into a render cache. `live_notes` selects the streaming
/// implementation: an instrument whose score carries a live note plays that note. An instrument
/// is stereo; an effect has its node's resolved layout (`ctx.layout`), never dual mono, so one
/// plugin instance processes both channels.
pub(super) fn translate_plugin(
    ctx: &mut NodeCtx<'_, '_>,
    live_notes: bool,
) -> Result<NodeValue, TranslateError> {
    if ctx.output_port != "audio" {
        return Err(ctx.missing_port());
    }
    let node = ctx.node;
    let factory = plugin_factory(node, ctx.bindings.plugins)?;
    let mode = if ctx.bindings.live().is_some() {
        RenderMode::Realtime
    } else {
        RenderMode::Offline
    };
    let (events, input, layout) = if node.node_type == PLUGIN_INSTRUMENT {
        let score = ctx.score("score")?;
        let events = match (&score.live, live_notes) {
            (Some(live), true) => EventSource::Live(Arc::clone(live)),
            _ => EventSource::Score(NoteEventSource::from_score_signal(&score, ctx.sample_rate)),
        };
        (events, None, Layout::Stereo)
    } else {
        let input = match ctx.layout {
            Layout::Mono => AudioSignal::Mono(ctx.signal("audio")?),
            Layout::Stereo => AudioSignal::Stereo(ctx.stereo_signal("audio")?),
        };
        (EventSource::None, Some(input), ctx.layout)
    };
    let stage = plugin_stage(node, factory.as_ref(), ctx.sample_rate, mode, layout)?;
    let cache = RenderCache::new(factory.plugin_id(), stage, events, input, ctx.sample_rate);
    let instances = &ctx.bindings.plugin_instances;
    instances.set(instances.get() + 1);
    Ok(match layout {
        Layout::Mono => NodeValue::Signal(Box::new(cache)),
        Layout::Stereo => NodeValue::Stereo(Box::new(cache)),
    })
}

/// A plugin node's deterministic implementation.
pub(super) fn translate_plugin_deterministic(
    ctx: &mut NodeCtx<'_, '_>,
) -> Result<NodeValue, TranslateError> {
    translate_plugin(ctx, false)
}

#[cfg(test)]
mod tests {
    use super::super::{translate_graph_with_options, TranslateOptions};
    use super::*;
    use crate::translate::StreamingProgram;
    use aura_clap::ClapHost;
    use aura_clap_fixture::{gain_state, FixtureEntry, GAIN_ID, SINE_ID};
    use aura_imp::{
        aura_registry, aura_registry_with_files,
        imp_file_from_json_path, Graph, NodeId, Registry, NOTATION_NODE_TYPE,
    };
    use aura_plugin_host::PluginResolver;
    use aura_imp::plugin::PLUGIN_EFFECT;
    use aura_render::BlockSource;
    use aura_sequencing::{LiveNote, NoteEvent, Score, ScoreSignal, Semitone, Tempo, TimeSignature};
    use clack_host::prelude::PluginEntry;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const RATE: SampleRate = SampleRate::RATE_44100;

    fn fixture_host() -> Arc<ClapHost> {
        let host = ClapHost::new(Vec::new());
        let entry =
            PluginEntry::load_from_clack::<FixtureEntry>(c"/fixture/aura_clap_fixture.clap")
                .expect("fixture entry");
        host.add_entry(entry, "/fixture/aura_clap_fixture.clap");
        Arc::new(host)
    }

    /// `notation → [voice →] [plugin_instrument →] plugin_effect* → out`, built from `chain`:
    /// node ids with `(type, plugin id, base64 state)`; the first consumes the score.
    fn graph(first_consumes: &str, chain: &[(&str, &str, &str, &str)]) -> Graph {
        let mut nodes = vec![
            format!(r#""n": {{ "id": "n", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }}"#),
            r#""t": { "id": "t", "type": "time", "inputs": {} }"#.to_string(),
            r#""o": { "id": "o", "type": "output", "inputs": {}, "title": "out" }"#.to_string(),
        ];
        let mut edges = Vec::new();
        let mut previous: Option<&str> = None;
        for (index, (id, node_type, plugin, state)) in chain.iter().enumerate() {
            let inputs = if plugin.is_empty() {
                "{}".to_string()
            } else {
                format!(r#"{{ "plugin": "{plugin}", "state": "{state}" }}"#)
            };
            nodes.push(format!(
                r#""{id}": {{ "id": "{id}", "type": "{node_type}", "inputs": {inputs} }}"#
            ));
            match previous {
                None => {
                    edges.push(format!(
                        r#""e{index}": {{ "id": "e{index}", "from": {{ "node": "n", "port": "score" }}, "to": {{ "node": "{id}", "port": "{first_consumes}" }} }}"#
                    ));
                    if *node_type == "sine_note_voice" {
                        edges.push(format!(
                            r#""et": {{ "id": "et", "from": {{ "node": "t", "port": "time" }}, "to": {{ "node": "{id}", "port": "time" }} }}"#
                        ));
                    }
                }
                Some(prev) => edges.push(format!(
                    r#""e{index}": {{ "id": "e{index}", "from": {{ "node": "{prev}", "port": "audio" }}, "to": {{ "node": "{id}", "port": "audio" }} }}"#
                )),
            }
            previous = Some(id);
        }
        let last = previous.expect("chain");
        edges.push(format!(
            r#""eo": {{ "id": "eo", "from": {{ "node": "{last}", "port": "audio" }}, "to": {{ "node": "o", "port": "value" }} }}"#
        ));
        let json = format!(
            r#"{{ "nodes": {{ {} }}, "edges": {{ {} }} }}"#,
            nodes.join(", "),
            edges.join(", ")
        );
        serde_json::from_str::<Graph>(&json).expect("graph")
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn sine_then_gain(gain: f32) -> Graph {
        let state = b64(&gain_state(gain));
        graph(
            "score",
            &[
                ("synth", PLUGIN_INSTRUMENT, SINE_ID, ""),
                ("fx", PLUGIN_EFFECT, GAIN_ID, &state),
            ],
        )
    }

    /// 120 bpm: A4 from 0.5 s to 1.0 s.
    fn bindings() -> BTreeMap<NodeId, ScoreSignal> {
        BTreeMap::from([(
            "n".to_string(),
            ScoreSignal::finite(Score::new(
                TimeSignature::default(),
                Tempo::new(120.0).unwrap(),
                vec![NoteEvent::new(Semitone(69), 1.0, 1.0)],
            )),
        )])
    }

    fn translate(
        graph: &Graph,
        plugins: Option<&dyn PluginResolver>,
    ) -> Result<AudioSignal, TranslateError> {
        let bindings = bindings();
        translate_graph_with_options(
            graph,
            &aura_registry().expect("registry"),
            RATE,
            TranslateOptions {
                score_bindings: &bindings,
                plugins,
            },
        )
    }

    /// Peak of either channel over `frames`.
    fn peak(signal: &mut AudioSignal, frames: std::ops::Range<u64>) -> f32 {
        aura_render::render_range(signal, frames.start, (frames.end - frames.start) as usize)
            .into_iter()
            .flatten()
            .fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn deterministic_plugin_chain_renders_the_score_reproducibly() {
        let host = fixture_host();
        let graph = sine_then_gain(0.5);
        let mut signal = translate(&graph, Some(host.as_ref())).expect("translate");
        assert_eq!(peak(&mut signal, 0..22051), 0.0, "silent before the note");
        let loud = peak(&mut signal, 22050..44100);
        assert!((loud - 0.125).abs() < 0.01, "sine 0.25 through gain 0.5: {loud}");
        assert_eq!(peak(&mut signal, 49000..60000), 0.0, "released");

        let mut again = translate(&graph, Some(host.as_ref())).expect("translate");
        let expected = aura_render::render_range(&mut signal, 0, 50_000);
        assert_eq!(aura_render::render_range(&mut again, 0, 50_000), expected);
    }

    #[test]
    fn plugin_chains_render_any_range_in_any_order_like_a_sequential_pass() {
        let host = fixture_host();
        let graph = sine_then_gain(0.5);
        let mut sequential = translate(&graph, Some(host.as_ref())).expect("translate");
        let expected = aura_render::render_range(&mut sequential, 0, 50_000);
        let mut sparse = translate(&graph, Some(host.as_ref())).expect("translate");
        for start in (0..49_000usize).rev().step_by(1_013) {
            let got = aura_render::render_range(&mut sparse, start as u64, 777);
            assert_eq!(got[..], expected[start..start + 777], "range at {start}");
        }
    }

    #[test]
    fn plugin_nodes_need_a_host_an_id_and_valid_state() {
        let graph = sine_then_gain(0.5);
        assert_eq!(
            translate(&graph, None).err(),
            Some(TranslateError::NoPluginHost {
                node: "fx".into(),
                plugin: GAIN_ID.into()
            })
        );
        let host = fixture_host();
        let no_id = graph_with_inputs(r#"{ "plugin": "" }"#);
        assert_eq!(
            translate(&no_id, Some(host.as_ref())).err(),
            Some(TranslateError::MissingPluginId {
                node: "synth".into()
            })
        );
        let bad_state = graph_with_inputs(&format!(r#"{{ "plugin": "{SINE_ID}", "state": "%%" }}"#));
        assert_eq!(
            translate(&bad_state, Some(host.as_ref())).err(),
            Some(TranslateError::InvalidPluginState {
                node: "synth".into()
            })
        );
        let missing = graph_with_inputs(r#"{ "plugin": "com.example.missing" }"#);
        assert!(matches!(
            translate(&missing, Some(host.as_ref())).err(),
            Some(TranslateError::Plugin { node, .. }) if node == "synth"
        ));
    }

    #[test]
    fn embedded_params_set_the_plugin_and_bad_params_fail_translation() {
        let host = fixture_host();
        let quiet = graph_with_inputs(&format!(
            r#"{{ "plugin": "{SINE_ID}", "params": "{{\"0\": 0.125}}" }}"#
        ));
        let mut signal = translate(&quiet, Some(host.as_ref())).expect("translate");
        let loud = peak(&mut signal, 22050..44100);
        assert!((loud - 0.125).abs() < 0.01, "level 0.125 applied: {loud}");

        for (params, reason) in [
            (r#"[1, 2]"#, "expected a JSON object"),
            (r#"{\"level\": 1}"#, "is not a parameter id"),
            (r#"{\"0\": \"loud\"}"#, "needs a finite number"),
        ] {
            let graph = graph_with_inputs(&format!(
                r#"{{ "plugin": "{SINE_ID}", "params": "{params}" }}"#
            ));
            match translate(&graph, Some(host.as_ref())).err() {
                Some(TranslateError::InvalidPluginParams { node, reason: got }) => {
                    assert_eq!(node, "synth");
                    assert!(got.contains(reason), "{params}: {got}");
                }
                other => panic!("{params}: expected invalid params, got {other:?}"),
            }
        }
        let unknown = graph_with_inputs(&format!(
            r#"{{ "plugin": "{SINE_ID}", "params": "{{\"7\": 1}}" }}"#
        ));
        assert_eq!(
            translate(&unknown, Some(host.as_ref())).err(),
            Some(TranslateError::Plugin {
                node: "synth".into(),
                source: aura_plugin_host::PluginHostError::UnknownParam {
                    id: SINE_ID.into(),
                    param: 7
                }
            })
        );
    }

    fn graph_with_inputs(inputs: &str) -> Graph {
        let json = format!(
            r#"{{
              "nodes": {{
                "n": {{ "id": "n", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }},
                "synth": {{ "id": "synth", "type": "{PLUGIN_INSTRUMENT}", "inputs": {inputs} }},
                "o": {{ "id": "o", "type": "output", "inputs": {{}}, "title": "out" }}
              }},
              "edges": {{
                "e1": {{ "id": "e1", "from": {{ "node": "n", "port": "score" }}, "to": {{ "node": "synth", "port": "score" }} }},
                "e2": {{ "id": "e2", "from": {{ "node": "synth", "port": "audio" }}, "to": {{ "node": "o", "port": "value" }} }}
              }}
            }}"#
        );
        serde_json::from_str::<Graph>(&json).expect("graph")
    }

    fn block_peak(source: &mut dyn BlockSource, blocks: usize) -> f32 {
        let mut block = vec![[0.0; 2]; 512];
        let mut peak = 0.0_f32;
        for _ in 0..blocks {
            source.render_block(&mut block);
            peak = block.iter().flatten().fold(peak, |m, s| m.max(s.abs()));
        }
        peak
    }

    fn live(semitone: i16) -> Arc<LiveNote> {
        Arc::new(LiveNote::new(Semitone(semitone), 1.0, Default::default()))
    }

    #[test]
    fn streaming_shares_one_instrument_instance_across_notes() {
        let host: Arc<dyn PluginResolver> = fixture_host();
        let program = StreamingProgram::prepare_with_plugins(
            &sine_then_gain(1.0),
            &aura_registry().expect("registry"),
            RATE,
            Some(host),
        )
        .expect("prepare");
        assert!(program.shares_plugins());
        let (control, mut source) = program.open().expect("open");
        assert_eq!(block_peak(source.as_mut(), 4), 0.0, "silent before a note");

        control.note_on(1, live(69)).unwrap();
        control.note_on(2, live(76)).unwrap();
        let chord = block_peak(source.as_mut(), 8);
        assert!(chord > 0.3, "two shared voices sound together: {chord}");

        control.note_off(1).unwrap();
        control.note_off(2).unwrap();
        // 0.1 s release plus the fixture's 64-frame latency.
        block_peak(source.as_mut(), 12);
        assert_eq!(block_peak(source.as_mut(), 4), 0.0, "rings out after release");
    }

    fn pack_registry() -> Registry {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../lab-pack/catalogs/instruments.json");
        let library = imp_file_from_json_path(&path).expect("instruments library");
        aura_registry_with_files(&[library]).expect("registry")
    }

    #[test]
    fn streaming_shares_an_effect_over_per_note_voices() {
        let host: Arc<dyn PluginResolver> = fixture_host();
        let state = b64(&gain_state(0.5));
        let with_gain = graph(
            "score",
            &[
                ("voice", "sine_note_voice", "", ""),
                ("fx", PLUGIN_EFFECT, GAIN_ID, &state),
            ],
        );
        let program = StreamingProgram::prepare_with_plugins(
            &with_gain,
            &pack_registry(),
            RATE,
            Some(host),
        )
        .expect("prepare");
        assert!(program.shares_plugins());
        let (control, mut source) = program.open().expect("open");
        control.note_on(1, live(69)).unwrap();
        let through_gain = block_peak(source.as_mut(), 8);

        let dry = StreamingProgram::prepare(
            &graph("score", &[("voice", "sine_note_voice", "", "")]),
            &pack_registry(),
            RATE,
        )
        .expect("dry");
        assert!(!dry.shares_plugins());
        let (dry_control, mut dry_source) = dry.open().expect("open");
        dry_control.note_on(1, live(69)).unwrap();
        let direct = block_peak(dry_source.as_mut(), 8);
        assert!(
            (through_gain - direct * 0.5).abs() < 0.02,
            "gain halves the mix: {through_gain} vs {direct}"
        );
    }

    /// `synth → multiply`: the instrument is stereo, so `multiply` resolves to stereo and runs as
    /// dual mono. Both channel passes share the one translated synth.
    #[test]
    fn dual_mono_over_a_plugin_shares_one_instance() {
        let host = fixture_host();
        let json = format!(
            r#"{{
              "nodes": {{
                "n": {{ "id": "n", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }},
                "synth": {{ "id": "synth", "type": "{PLUGIN_INSTRUMENT}", "inputs": {{ "plugin": "{SINE_ID}" }} }},
                "gain": {{ "id": "gain", "type": "multiply", "inputs": {{ "b": 0.5 }} }},
                "o": {{ "id": "o", "type": "output", "inputs": {{}} }}
              }},
              "edges": {{
                "e1": {{ "id": "e1", "from": {{ "node": "n", "port": "score" }}, "to": {{ "node": "synth", "port": "score" }} }},
                "e2": {{ "id": "e2", "from": {{ "node": "synth", "port": "audio" }}, "to": {{ "node": "gain", "port": "a" }} }},
                "e3": {{ "id": "e3", "from": {{ "node": "gain", "port": "audio" }}, "to": {{ "node": "o", "port": "value" }} }}
              }}
            }}"#
        );
        let graph = serde_json::from_str::<Graph>(&json).expect("graph");
        let registry = aura_registry().expect("registry");
        let scores = bindings();
        let prepared = super::super::PreparedGraph::prepare(&graph, &registry).expect("prepare");
        let translation = super::super::Bindings::new(
            &scores,
            super::super::TranslateMode::Deterministic,
            Some(host.as_ref()),
        );
        let mut signal = prepared
            .translate(&registry, RATE, &translation)
            .expect("translate");
        assert_eq!(signal.layout(), aura_render::Layout::Stereo);
        assert_eq!(translation.plugin_instances.get(), 1, "one plugin instance");
        let loud = peak(&mut signal, 22050..44100);
        assert!((loud - 0.125).abs() < 0.01, "sine 0.25 at gain 0.5: {loud}");
    }

    /// `synth` feeds two gains that are summed: translation builds the synth once and both
    /// gains read the same rendered blocks.
    #[test]
    fn fan_out_builds_one_plugin_instance() {
        let host = fixture_host();
        let json = format!(
            r#"{{
              "nodes": {{
                "n": {{ "id": "n", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }},
                "synth": {{ "id": "synth", "type": "{PLUGIN_INSTRUMENT}", "inputs": {{ "plugin": "{SINE_ID}" }} }},
                "half": {{ "id": "half", "type": "multiply", "inputs": {{ "b": 0.5 }} }},
                "quarter": {{ "id": "quarter", "type": "multiply", "inputs": {{ "b": 0.25 }} }},
                "mix": {{ "id": "mix", "type": "add", "inputs": {{}} }},
                "o": {{ "id": "o", "type": "output", "inputs": {{}} }}
              }},
              "edges": {{
                "e1": {{ "id": "e1", "from": {{ "node": "n", "port": "score" }}, "to": {{ "node": "synth", "port": "score" }} }},
                "e2": {{ "id": "e2", "from": {{ "node": "synth", "port": "audio" }}, "to": {{ "node": "half", "port": "a" }} }},
                "e3": {{ "id": "e3", "from": {{ "node": "synth", "port": "audio" }}, "to": {{ "node": "quarter", "port": "a" }} }},
                "e4": {{ "id": "e4", "from": {{ "node": "half", "port": "audio" }}, "to": {{ "node": "mix", "port": "inputs" }}, "order": 0 }},
                "e5": {{ "id": "e5", "from": {{ "node": "quarter", "port": "audio" }}, "to": {{ "node": "mix", "port": "inputs" }}, "order": 1 }},
                "e6": {{ "id": "e6", "from": {{ "node": "mix", "port": "audio" }}, "to": {{ "node": "o", "port": "value" }} }}
              }}
            }}"#
        );
        let graph = serde_json::from_str::<Graph>(&json).expect("graph");
        let registry = aura_registry().expect("registry");
        let scores = bindings();
        let prepared = super::super::PreparedGraph::prepare(&graph, &registry).expect("prepare");
        let translation = super::super::Bindings::new(
            &scores,
            super::super::TranslateMode::Deterministic,
            Some(host.as_ref()),
        );
        let mut signal = prepared
            .translate(&registry, RATE, &translation)
            .expect("translate");
        assert_eq!(translation.plugin_instances.get(), 1, "one plugin instance");
        let loud = peak(&mut signal, 22050..44100);
        assert!((loud - 0.1875).abs() < 0.01, "sine 0.25 at gain 0.75: {loud}");
    }

    #[test]
    fn per_voice_fallback_plays_the_live_note() {
        let host: Arc<dyn PluginResolver> = fixture_host();
        let program = StreamingProgram::prepare_with_plugins(
            &sine_then_gain(1.0),
            &aura_registry().expect("registry"),
            RATE,
            Some(host),
        )
        .expect("prepare");
        let note = live(69);
        let mut voice = program.instantiate(Arc::clone(&note)).expect("voice");
        let held = peak(&mut voice, 100..4000);
        assert!(held > 0.2, "held note sounds: {held}");
        note.release(4000.0 / 44100.0);
        assert_eq!(peak(&mut voice, 4000 + 6000..20000), 0.0, "released");
    }
}
