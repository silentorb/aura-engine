//! Imp Lab `service` extension: Imp graph + Aura notation playback via CPAL.
//!
//! Transport playback (`start` / `stop` / `toggle`) renders in deterministic mode. Live note
//! audition (`event`: note on / off) renders in streaming mode; see [`audition`]. MIDI input
//! ([`midi_input`]) is monitored the same way and recorded into notation ([`recording`]).

pub mod audition;
pub mod midi_input;
pub mod plugins;
pub mod recording;

use aura_clap::ClapHost;
use aura_imp::{
    aura_registry_with_libraries, discover_graph_libraries, graph_from_json_str,
    graph_library_from_json_path, Graph, GraphLibrary, Registry,
};
use aura_integration::{
    find_output_node, sampler_from_score, score_from_notation_str, translate_graph_with_project,
};
use aura_io_cpal::PlaybackEngine;
use aura_render::{AudioSampler, Sampler};
use aura_sample::SampleRate;
use aura_sequencing::beats_to_seconds;
use imp_lab_extensions::{
    write_service_error, write_service_reply, ImpLabHostV0, ImpLabServiceV0, Publisher,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::raw::c_uchar;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use thiserror::Error;

/// Whether `path` is a Tome v2 project config (as opposed to a legacy graph file).
pub fn is_project_config(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<Value>(&text).ok().is_some_and(|v| {
        v.get("version") == Some(&Value::from(2)) && v.get("dataStores").is_some()
    })
}

pub(crate) const SAMPLE_RATE: SampleRate = SampleRate::RATE_44100;
const SERVICE_IMPLEMENTATION_ID: &str = "playback";

struct PlaybackState {
    engine: Mutex<PlaybackEngine>,
    /// When set, auto-stop once `position_secs` reaches this value (notation audition).
    end_secs: Mutex<Option<f64>>,
    /// Added to the engine clock to give notation time (non-zero while recording, whose
    /// transport starts at the count-in).
    position_offset: Mutex<f64>,
}

fn state() -> &'static PlaybackState {
    static STATE: OnceLock<PlaybackState> = OnceLock::new();
    STATE.get_or_init(|| PlaybackState {
        engine: Mutex::new(PlaybackEngine::new()),
        end_secs: Mutex::new(None),
        position_offset: Mutex::new(0.0),
    })
}

fn set_position_offset(offset: f64) -> Result<(), PlaybackServiceError> {
    *state()
        .position_offset
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)? = offset;
    Ok(())
}

/// Topic publisher kept from registration (absent in tests and older hosts).
fn publisher() -> &'static OnceLock<Publisher> {
    static PUBLISHER: OnceLock<Publisher> = OnceLock::new();
    &PUBLISHER
}

/// Publishes a topic value through the host, when it offers publishing.
pub(crate) fn publish<T: Serialize + ?Sized>(topic: &str, value: &T) {
    if let Some(publisher) = publisher().get() {
        publisher.publish(topic, value);
    }
}

/// Starts the transport on `sampler` from its `t = 0`, which is notation time `offset_secs`.
pub(crate) fn start_transport(
    sampler: Arc<AudioSampler>,
    offset_secs: f64,
) -> Result<(), PlaybackServiceError> {
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    engine
        .start(sampler, SAMPLE_RATE)
        .map_err(|e| PlaybackServiceError::Playback(e.to_string()))?;
    drop(engine);
    set_end_secs(None)?;
    set_position_offset(offset_secs)
}

/// Stops the transport without touching a recording take.
pub(crate) fn stop_transport() {
    if let Ok(mut engine) = state().engine.lock() {
        engine.stop();
    }
    let _ = set_end_secs(None);
    let _ = set_position_offset(0.0);
}

/// Errors preparing or running Lab pack playback.
#[derive(Debug, Error)]
pub enum PlaybackServiceError {
    #[error("invalid graph JSON: {0}")]
    GraphJson(String),
    #[error("invalid notation JSON: {0}")]
    NotationJson(String),
    #[error("{0}")]
    Output(String),
    #[error("registry: {0}")]
    Registry(String),
    #[error("project: {0}")]
    Project(String),
    #[error("translate: {0}")]
    Translate(String),
    #[error("playback: {0}")]
    Playback(String),
    #[error("event: {0}")]
    Event(String),
    #[error("request: {0}")]
    Request(String),
    #[error("recording: {0}")]
    Recording(String),
    #[error("internal lock poisoned")]
    Poisoned,
}

/// Graph libraries shipped in the Lab pack (`catalogs/instruments.json`), found at register.
fn pack_library_paths() -> &'static Mutex<Vec<PathBuf>> {
    static PATHS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

fn pack_libraries() -> Result<Vec<GraphLibrary>, PlaybackServiceError> {
    let paths = pack_library_paths()
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?
        .clone();
    paths
        .iter()
        .map(|p| {
            graph_library_from_json_path(p)
                .map_err(|e| PlaybackServiceError::Registry(e.to_string()))
        })
        .collect()
}

/// The CLAP host that resolves plugin nodes, searching the platform's standard CLAP locations
/// (and `CLAP_PATH`). Bundles load on first use and stay loaded for the process.
pub fn plugin_host() -> &'static Arc<ClapHost> {
    static HOST: OnceLock<Arc<ClapHost>> = OnceLock::new();
    HOST.get_or_init(|| Arc::new(ClapHost::with_standard_paths(Vec::new())))
}

/// Builds a registry for playback. A legacy graph-file hint discovers graph libraries next to
/// the file; a project config (or no hint) uses the Lab pack's libraries.
pub fn playback_registry(neighbor_hint: Option<&Path>) -> Result<Registry, PlaybackServiceError> {
    let libraries = match neighbor_hint {
        Some(path) if !is_project_config(path) => discover_graph_libraries(path)
            .map_err(|e| PlaybackServiceError::Registry(e.to_string()))?,
        _ => pack_libraries()?,
    };
    aura_registry_with_libraries(&libraries)
        .map_err(|e| PlaybackServiceError::Registry(e.to_string()))
}

/// Parse graph JSON from the Lab host (bare Imp graph or tip-wrapped).
pub fn parse_graph_json(json: &str) -> Result<Graph, PlaybackServiceError> {
    match graph_from_json_str(json) {
        Ok(graph) => Ok(graph),
        Err(first) => {
            let value: Value = serde_json::from_str(json)
                .map_err(|e| PlaybackServiceError::GraphJson(e.to_string()))?;
            let nodes = value.get("nodes").cloned();
            let edges = value.get("edges").cloned();
            match (nodes, edges) {
                (Some(nodes), Some(edges)) => {
                    let bare = serde_json::json!({ "nodes": nodes, "edges": edges });
                    let text = serde_json::to_string(&bare)
                        .map_err(|e| PlaybackServiceError::GraphJson(e.to_string()))?;
                    graph_from_json_str(&text)
                        .map_err(|e| PlaybackServiceError::GraphJson(e.to_string()))
                }
                _ => Err(PlaybackServiceError::GraphJson(first.to_string())),
            }
        }
    }
}

/// Translate graph JSON into a sampler ready for CPAL.
pub fn prepare_graph_playback(
    graph_json: &str,
    neighbor_hint: Option<&Path>,
) -> Result<(String, AudioSampler), PlaybackServiceError> {
    let graph = parse_graph_json(graph_json)?;
    // Nodes the graph references but does not hold (a notation, a node of another graph) come
    // from the project's Tome stores. A legacy graph-file hint resolves nothing beyond the file.
    let project = neighbor_hint.filter(|p| is_project_config(p));
    let node_id =
        find_output_node(&graph).map_err(|e| PlaybackServiceError::Output(e.to_string()))?;
    let registry = playback_registry(neighbor_hint)?;
    let sampler =
        translate_graph_with_project(&graph, &registry, SAMPLE_RATE, project, Some(plugin_host().as_ref()))
            .map_err(|e| PlaybackServiceError::Translate(e.to_string()))?;
    Ok((node_id, sampler))
}

fn payload_type(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json).ok()?;
    value.get("type")?.as_str().map(str::to_string)
}

fn lab_start_beats(json: &str) -> f64 {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return 0.0;
    };
    value
        .get("labStartBeats")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(0.0)
}

/// Prepare notation tip JSON into a sampler + start/end times.
pub fn prepare_notation_playback(
    notation_json: &str,
) -> Result<(Box<dyn Sampler>, f64, f64), PlaybackServiceError> {
    let score =
        score_from_notation_str(notation_json).map_err(PlaybackServiceError::NotationJson)?;
    let duration_secs = score.duration_secs().max(0.0);
    let start_beats = lab_start_beats(notation_json);
    let tempo = score.tempo;
    let start_secs = beats_to_seconds(start_beats.max(0.0), tempo);
    let sampler = sampler_from_score(score, SAMPLE_RATE);
    Ok((sampler, start_secs, duration_secs.max(start_secs)))
}

fn set_end_secs(end: Option<f64>) -> Result<(), PlaybackServiceError> {
    let mut guard = state()
        .end_secs
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    *guard = end;
    Ok(())
}

fn maybe_auto_stop(engine: &mut PlaybackEngine) -> Result<(), PlaybackServiceError> {
    let end = {
        let guard = state()
            .end_secs
            .lock()
            .map_err(|_| PlaybackServiceError::Poisoned)?;
        *guard
    };
    if let Some(end_secs) = end {
        if engine.is_playing() && engine.position_secs() >= end_secs {
            engine.stop();
            set_end_secs(None)?;
        }
    }
    Ok(())
}

/// Start (or restart) playback of graph or notation JSON.
pub fn start_playback(
    payload_json: &str,
    neighbor_hint: Option<&Path>,
) -> Result<(), PlaybackServiceError> {
    recording::end_if_active();
    set_position_offset(0.0)?;
    match payload_type(payload_json).as_deref() {
        Some("notation") => {
            let (sampler, start_secs, end_secs) = prepare_notation_playback(payload_json)?;
            let mut engine = state()
                .engine
                .lock()
                .map_err(|_| PlaybackServiceError::Poisoned)?;
            engine
                .start_at(Arc::new(AudioSampler::Mono(sampler)), SAMPLE_RATE, start_secs)
                .map_err(|e| PlaybackServiceError::Playback(e.to_string()))?;
            drop(engine);
            set_end_secs(Some(end_secs))?;
            Ok(())
        }
        _ => {
            let (_node_id, sampler) = prepare_graph_playback(payload_json, neighbor_hint)?;
            let mut engine = state()
                .engine
                .lock()
                .map_err(|_| PlaybackServiceError::Poisoned)?;
            engine
                .start(Arc::new(sampler), SAMPLE_RATE)
                .map_err(|e| PlaybackServiceError::Playback(e.to_string()))?;
            drop(engine);
            set_end_secs(None)?;
            Ok(())
        }
    }
}

/// Stop playback if running.
pub fn stop_playback() -> Result<(), PlaybackServiceError> {
    // Stopping the transport ends a take; its notes wait for `record_stop`.
    recording::end_if_active();
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    engine.stop();
    drop(engine);
    set_end_secs(None)?;
    set_position_offset(0.0)?;
    Ok(())
}

/// Toggle: stop if playing, otherwise start.
pub fn toggle_playback(
    payload_json: &str,
    neighbor_hint: Option<&Path>,
) -> Result<(), PlaybackServiceError> {
    let playing = {
        let mut engine = state()
            .engine
            .lock()
            .map_err(|_| PlaybackServiceError::Poisoned)?;
        maybe_auto_stop(&mut engine)?;
        engine.is_playing()
    };
    if playing {
        stop_playback()
    } else {
        start_playback(payload_json, neighbor_hint)
    }
}

/// Whether CPAL playback is currently active (runs notation auto-stop).
pub fn is_playing() -> Result<bool, PlaybackServiceError> {
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    maybe_auto_stop(&mut engine)?;
    Ok(engine.is_playing())
}

/// Current playback position in seconds (runs notation auto-stop). While recording this is
/// notation time, held at 0 during a count-in that starts before the notation.
pub fn position_secs() -> Result<f64, PlaybackServiceError> {
    Ok(position_secs_unclamped()?.max(0.0))
}

/// Position on the notation timeline; negative during a count-in before beat 0.
pub(crate) fn position_secs_unclamped() -> Result<f64, PlaybackServiceError> {
    let offset = *state()
        .position_offset
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    maybe_auto_stop(&mut engine)?;
    Ok(engine.position_secs() + offset)
}

fn read_payload_and_path(
    graph_json: *const c_uchar,
    graph_len: usize,
    neighbor_path: *const c_char,
) -> Result<(String, Option<PathBuf>), PlaybackServiceError> {
    if graph_json.is_null() || graph_len == 0 {
        return Err(PlaybackServiceError::GraphJson(
            "playback JSON is required".into(),
        ));
    }
    let slice = unsafe { std::slice::from_raw_parts(graph_json, graph_len) };
    let text = std::str::from_utf8(slice)
        .map_err(|e| PlaybackServiceError::GraphJson(e.to_string()))?
        .to_string();
    let path = if neighbor_path.is_null() {
        None
    } else {
        let c = unsafe { CStr::from_ptr(neighbor_path) };
        let s = c
            .to_str()
            .map_err(|e| PlaybackServiceError::GraphJson(e.to_string()))?;
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    };
    Ok((text, path))
}

fn fail(err_out: *mut c_char, err_len: usize, err: PlaybackServiceError) -> c_int {
    unsafe {
        write_service_error(err_out, err_len, &err.to_string());
    }
    -1
}

unsafe extern "C" fn service_start(
    _ctx: *mut c_void,
    graph_json: *const c_uchar,
    graph_len: usize,
    neighbor_path: *const c_char,
    err_out: *mut c_char,
    err_len: usize,
) -> c_int {
    match read_payload_and_path(graph_json, graph_len, neighbor_path) {
        Ok((json, path)) => match start_playback(&json, path.as_deref()) {
            Ok(()) => 0,
            Err(e) => fail(err_out, err_len, e),
        },
        Err(e) => fail(err_out, err_len, e),
    }
}

unsafe extern "C" fn service_stop(
    _ctx: *mut c_void,
    err_out: *mut c_char,
    err_len: usize,
) -> c_int {
    match stop_playback() {
        Ok(()) => 0,
        Err(e) => fail(err_out, err_len, e),
    }
}

unsafe extern "C" fn service_toggle(
    _ctx: *mut c_void,
    graph_json: *const c_uchar,
    graph_len: usize,
    neighbor_path: *const c_char,
    err_out: *mut c_char,
    err_len: usize,
) -> c_int {
    match read_payload_and_path(graph_json, graph_len, neighbor_path) {
        Ok((json, path)) => match toggle_playback(&json, path.as_deref()) {
            Ok(()) => 0,
            Err(e) => fail(err_out, err_len, e),
        },
        Err(e) => fail(err_out, err_len, e),
    }
}

unsafe extern "C" fn service_is_playing(_ctx: *mut c_void) -> c_int {
    match is_playing() {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

unsafe extern "C" fn service_position_secs(_ctx: *mut c_void) -> f64 {
    position_secs().unwrap_or(0.0)
}

unsafe extern "C" fn service_event(
    _ctx: *mut c_void,
    json: *const c_uchar,
    len: usize,
    err_out: *mut c_char,
    err_len: usize,
) -> c_int {
    if json.is_null() || len == 0 {
        let err = PlaybackServiceError::Event("event JSON is required".into());
        return fail(err_out, err_len, err);
    }
    // SAFETY: the host passes a valid buffer of `len` bytes for the duration of the call.
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    let result = std::str::from_utf8(bytes)
        .map_err(|e| PlaybackServiceError::Event(e.to_string()))
        .and_then(handle_event);
    match result {
        Ok(()) => 0,
        Err(e) => fail(err_out, err_len, e),
    }
}

/// Applies a service event: `settings` (extension setting values) or a note audition event.
pub fn handle_event(json: &str) -> Result<(), PlaybackServiceError> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| PlaybackServiceError::Event(e.to_string()))?;
    if value.get("type").and_then(Value::as_str) == Some("settings") {
        let values = value
            .get("values")
            .and_then(Value::as_object)
            .ok_or_else(|| PlaybackServiceError::Event("settings needs a `values` object".into()))?;
        midi_input::apply_settings(values);
        return Ok(());
    }
    audition::handle_event(json)
}

fn request_bool(value: &Value, key: &str, default: bool) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// Answers a service request (the live note capture protocol):
///
/// - `{"type":"capture_info"}` → `{"topic"}`: the take topic (and that capture is offered)
/// - `{"type":"record_start","notation":{…},"startBeats":n,"click":bool}` → `{"topic"}`
/// - `{"type":"record_stop"}` → `{"notes":[…]}` (notation events)
/// - `{"type":"node_action","action":"plugin.open_gui","node":{…}}` → `{}`: opens the node's plugin
///   GUI ([`plugins`])
pub fn handle_request(json: &str) -> Result<String, PlaybackServiceError> {
    let bad = |message: &str| PlaybackServiceError::Request(message.to_string());
    let value: Value = serde_json::from_str(json).map_err(|e| bad(&e.to_string()))?;
    let reply = match value.get("type").and_then(Value::as_str) {
        Some("capture_info") => json!({ "topic": recording::TOPIC_TAKE }),
        Some("record_start") => {
            let notation = value
                .get("notation")
                .ok_or_else(|| bad("record_start needs `notation`"))?;
            let start_beats = value
                .get("startBeats")
                .and_then(Value::as_f64)
                .filter(|b| b.is_finite())
                .unwrap_or(0.0);
            recording::record_start(
                &notation.to_string(),
                start_beats,
                request_bool(&value, "click", true),
            )?;
            json!({ "topic": recording::TOPIC_TAKE })
        }
        Some("record_stop") => json!({ "notes": recording::notes_json(&recording::record_stop()?) }),
        Some("node_action") => plugins::node_action(&value)?,
        Some(other) => return Err(bad(&format!("unknown request type `{other}`"))),
        None => return Err(bad("request needs a string `type`")),
    };
    Ok(reply.to_string())
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn service_request(
    _ctx: *mut c_void,
    json: *const c_uchar,
    len: usize,
    out: *mut c_uchar,
    out_cap: usize,
    out_len: *mut usize,
    err_out: *mut c_char,
    err_len: usize,
) -> c_int {
    if json.is_null() || len == 0 {
        let err = PlaybackServiceError::Request("request JSON is required".into());
        return fail(err_out, err_len, err);
    }
    // SAFETY: the host passes a valid buffer of `len` bytes for the duration of the call.
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    let result = std::str::from_utf8(bytes)
        .map_err(|e| PlaybackServiceError::Request(e.to_string()))
        .and_then(handle_request);
    match result {
        // SAFETY: the host passes an `out` buffer of `out_cap` bytes and a writable `out_len`.
        Ok(reply) => unsafe { write_service_reply(out, out_cap, out_len, &reply) },
        Err(e) => fail(err_out, err_len, e),
    }
}

/// Record the pack's demo instruments library (same search as the catalogs extension) so
/// project playback can build voices like `sine_note_voice`.
fn remember_pack_libraries(host: &ImpLabHostV0) {
    const CANDIDATES: &[&str] = &[
        "catalogs/instruments.json",
        "instruments.json",
        "demos/instruments.json",
    ];
    let Some(resolve) = host.resolve_pack_path else {
        return;
    };
    let found = CANDIDATES.iter().find_map(|rel| {
        let rel_c = CString::new(*rel).ok()?;
        let mut buf = vec![0u8; 4096];
        // SAFETY: host vtable function with a valid C string and a sized output buffer.
        let code = unsafe {
            resolve(
                host.ctx,
                rel_c.as_ptr(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            )
        };
        if code != 0 {
            return None;
        }
        // SAFETY: the host wrote a NUL-terminated path into `buf`.
        let path = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        let path = PathBuf::from(path.to_str().ok()?);
        path.is_file().then_some(path)
    });
    if let (Some(path), Ok(mut paths)) = (found, pack_library_paths().lock()) {
        *paths = vec![path];
    }
}

/// Lab extension entrypoint.
///
/// # Safety
///
/// `host` must be a valid pointer to an [`ImpLabHostV0`] for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn imp_lab_extension_register(host: *const ImpLabHostV0) -> i32 {
    if host.is_null() {
        return -1;
    }
    let h = &*host;
    let Some(register_service) = h.register_service else {
        return -1;
    };
    let _ = state();
    remember_pack_libraries(h);
    if let Some(host_publisher) = h.publisher() {
        let _ = publisher().set(host_publisher);
    }
    // Scans installed plugins off the startup path and publishes the plugin lists.
    plugins::publish_plugin_lists();
    // Publishes the MIDI port list and status, then keeps the configured device connected.
    midi_input::ensure_thread();
    let service = ImpLabServiceV0 {
        ctx: std::ptr::null_mut(),
        start: Some(service_start),
        stop: Some(service_stop),
        toggle: Some(service_toggle),
        is_playing: Some(service_is_playing),
        position_secs: Some(service_position_secs),
        event: Some(service_event),
        request: Some(service_request),
    };
    let id = match CString::new(SERVICE_IMPLEMENTATION_ID) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    register_service(h.ctx, id.as_ptr(), &service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imp_lab_extensions::{host_vtable, ExtensionHost};

    fn sine_graph_json() -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/sine.json");
        std::fs::read_to_string(path).expect("sine.json")
    }

    #[test]
    fn prepare_sine_graph_yields_sampler() {
        let json = sine_graph_json();
        let (node_id, sampler) = prepare_graph_playback(&json, None).expect("prepare");
        assert!(!node_id.is_empty());
        let sample = sampler.frame(0.01)[0];
        assert!(sample.abs() > 1e-6, "expected audible sample, got {sample}");
    }

    #[test]
    fn prepare_notation_yields_sampler() {
        let json = r#"{
            "type":"notation",
            "version":1,
            "tempoBpm":120,
            "timeSignature":{"beatsPerBar":4,"beatUnit":4},
            "labStartBeats":0.0,
            "events":[{"semitone":60,"startBeats":0.0,"durationBeats":0.5,"velocity":1.0}]
        }"#;
        let (sampler, start_secs, end_secs) = prepare_notation_playback(json).expect("prepare");
        assert_eq!(start_secs, 0.0);
        assert!(end_secs > 0.0);
        let sample = sampler.at(0.05);
        assert!(sample.abs() > 1e-4, "expected audible sample, got {sample}");
    }

    fn workspace_path(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    /// Points playback at the pack's instruments library, as registration does in Lab.
    pub(crate) fn use_pack_libraries() {
        *pack_library_paths().lock().unwrap() =
            vec![workspace_path("lab-pack/catalogs/instruments.json")];
    }

    #[test]
    fn project_playback_resolves_the_notation_record_demo_02_uses() {
        use_pack_libraries();
        let config = workspace_path("lab-pack/demo-project/imp-project.json");
        assert!(is_project_config(&config));
        assert!(!is_project_config(&workspace_path("demos/demo-02.json")));

        let json = std::fs::read_to_string(workspace_path("demos/demo-02.json")).unwrap();
        let (_node, sampler) =
            prepare_graph_playback(&json, Some(&config)).expect("demo-02 plays from the project");
        let peak = (0..200)
            .map(|i| sampler.frame(i as f64 * 0.01)[0].abs())
            .fold(0.0_f32, f32::max);
        assert!(peak > 1e-4, "expected audible output, peak {peak}");

        // Without the project the arpeggio notation cannot be found.
        assert!(prepare_graph_playback(&json, None).is_err());
    }

    #[test]
    fn register_installs_playback_service() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = ExtensionHost::new(dir.path());
        let vtable = host_vtable(&mut host);
        let code = unsafe { imp_lab_extension_register(&vtable) };
        assert_eq!(code, 0);
        assert!(host.services().get("playback").is_some());
    }

    #[test]
    fn capture_requests() {
        assert_eq!(
            handle_request(r#"{"type":"capture_info"}"#).unwrap(),
            r#"{"topic":"aura.capture.take"}"#
        );
        assert_eq!(
            handle_request(r#"{"type":"record_stop"}"#).unwrap(),
            r#"{"notes":[]}"#,
            "no take: no notes"
        );
        assert!(handle_request(r#"{"type":"record_start"}"#).is_err());
        assert!(handle_request(r#"{"type":"bogus"}"#).is_err());
        assert!(handle_request("nope").is_err());
    }

    #[test]
    fn settings_events_are_accepted() {
        assert!(handle_event(r#"{"type":"settings","values":{"aura.midi.channels":["1"]}}"#).is_ok());
        assert!(handle_event(r#"{"type":"settings"}"#).is_err());
    }

    #[test]
    fn pack_registers_midi_settings_that_resolve() {
        let text = std::fs::read_to_string(workspace_path("lab-pack/extensions.json")).unwrap();
        let config = imp_lab_extensions::parse_extensions_json(&text).expect("pack config");
        let layout = imp_lab_extensions::settings_layout(&config);
        assert!(layout.warnings.is_empty(), "{:?}", layout.warnings);
        let midi = layout
            .pages
            .iter()
            .find(|p| p.category_id.as_deref() == Some("aura.midi"))
            .expect("MIDI page");
        let ids: Vec<&str> = midi.settings.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec![midi_input::SETTING_INPUT, midi_input::SETTING_CHANNELS]);
        assert!(midi
            .settings
            .iter()
            .all(|s| s.service_id.as_deref() == Some(SERVICE_IMPLEMENTATION_ID)));
        let status = imp_lab_extensions::status_items(&config);
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].topic, midi_input::TOPIC_STATUS);
    }
}
