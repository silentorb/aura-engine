//! Imp Lab `service` extension: Imp graph + Aura notation playback via CPAL.

use aura_imp::{
    aura_registry_with_libraries, discover_graph_libraries, graph_from_json_str,
    graph_library_from_json_path, Graph, GraphLibrary, Registry,
};
use aura_integration::{
    find_output_node, sampler_from_score, score_from_notation_str, translate_graph_with_project,
};
use aura_io_cpal::PlaybackEngine;
use aura_render::Sampler;
use aura_sample::SampleRate;
use aura_sequencing::beats_to_seconds;
use imp_lab_extensions::{write_service_error, ImpLabHostV0, ImpLabServiceV0};
use serde_json::Value;
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

const SAMPLE_RATE: SampleRate = SampleRate::RATE_44100;
const SERVICE_IMPLEMENTATION_ID: &str = "playback";

struct PlaybackState {
    engine: Mutex<PlaybackEngine>,
    /// When set, auto-stop once `position_secs` reaches this value (notation audition).
    end_secs: Mutex<Option<f64>>,
}

fn state() -> &'static PlaybackState {
    static STATE: OnceLock<PlaybackState> = OnceLock::new();
    STATE.get_or_init(|| PlaybackState {
        engine: Mutex::new(PlaybackEngine::new()),
        end_secs: Mutex::new(None),
    })
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
) -> Result<(String, Box<dyn Sampler>), PlaybackServiceError> {
    let graph = parse_graph_json(graph_json)?;
    // Nodes the graph references but does not hold (a notation, a node of another graph) come
    // from the project's Tome stores. A legacy graph-file hint resolves nothing beyond the file.
    let project = neighbor_hint.filter(|p| is_project_config(p));
    let node_id =
        find_output_node(&graph).map_err(|e| PlaybackServiceError::Output(e.to_string()))?;
    let registry = playback_registry(neighbor_hint)?;
    let sampler = translate_graph_with_project(&graph, &registry, SAMPLE_RATE, project)
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
    match payload_type(payload_json).as_deref() {
        Some("notation") => {
            let (sampler, start_secs, end_secs) = prepare_notation_playback(payload_json)?;
            let mut engine = state()
                .engine
                .lock()
                .map_err(|_| PlaybackServiceError::Poisoned)?;
            engine
                .start_at(Arc::from(sampler), SAMPLE_RATE, start_secs)
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
                .start(Arc::from(sampler), SAMPLE_RATE)
                .map_err(|e| PlaybackServiceError::Playback(e.to_string()))?;
            drop(engine);
            set_end_secs(None)?;
            Ok(())
        }
    }
}

/// Stop playback if running.
pub fn stop_playback() -> Result<(), PlaybackServiceError> {
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    engine.stop();
    drop(engine);
    set_end_secs(None)?;
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

/// Current playback position in seconds (runs notation auto-stop).
pub fn position_secs() -> Result<f64, PlaybackServiceError> {
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    maybe_auto_stop(&mut engine)?;
    Ok(engine.position_secs())
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
    let service = ImpLabServiceV0 {
        ctx: std::ptr::null_mut(),
        start: Some(service_start),
        stop: Some(service_stop),
        toggle: Some(service_toggle),
        is_playing: Some(service_is_playing),
        position_secs: Some(service_position_secs),
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
        let sample = sampler.at(0.01);
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

    #[test]
    fn project_playback_resolves_the_notation_record_demo_02_uses() {
        *pack_library_paths().lock().unwrap() =
            vec![workspace_path("lab-pack/catalogs/instruments.json")];
        let config = workspace_path("lab-pack/demo-project/imp-project.json");
        assert!(is_project_config(&config));
        assert!(!is_project_config(&workspace_path("demos/demo-02.json")));

        let json = std::fs::read_to_string(workspace_path("demos/demo-02.json")).unwrap();
        let (_node, sampler) =
            prepare_graph_playback(&json, Some(&config)).expect("demo-02 plays from the project");
        let peak = (0..200)
            .map(|i| sampler.at(i as f64 * 0.01).abs())
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
}
