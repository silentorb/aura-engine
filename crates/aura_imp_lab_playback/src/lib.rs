//! Imp Lab `service` extension: Imp graph playback via Aura translate + CPAL.

use aura_imp::{
    aura_registry_with_libraries, discover_graph_libraries, graph_from_json_str, Graph, Registry,
};
use aura_integration::{
    find_output_node, translate_graph_to_sampler_with_options, TranslateOptions,
};
use aura_io_cpal::PlaybackEngine;
use aura_render::Sampler;
use aura_sample::SampleRate;
use imp_lab_extensions::{write_service_error, ImpLabHostV0, ImpLabServiceV0};
use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::raw::c_uchar;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use thiserror::Error;

const SAMPLE_RATE: SampleRate = SampleRate::RATE_44100;
const SERVICE_IMPLEMENTATION_ID: &str = "playback";

struct PlaybackState {
    engine: Mutex<PlaybackEngine>,
}

fn state() -> &'static PlaybackState {
    static STATE: OnceLock<PlaybackState> = OnceLock::new();
    STATE.get_or_init(|| PlaybackState {
        engine: Mutex::new(PlaybackEngine::new()),
    })
}

/// Errors preparing or running Lab pack playback.
#[derive(Debug, Error)]
pub enum PlaybackServiceError {
    #[error("invalid graph JSON: {0}")]
    GraphJson(String),
    #[error("{0}")]
    Output(String),
    #[error("registry: {0}")]
    Registry(String),
    #[error("translate: {0}")]
    Translate(String),
    #[error("playback: {0}")]
    Playback(String),
    #[error("internal lock poisoned")]
    Poisoned,
}

/// Builds a registry for playback, optionally discovering neighbor graph libraries.
pub fn playback_registry(neighbor_hint: Option<&Path>) -> Result<Registry, PlaybackServiceError> {
    let libraries = match neighbor_hint {
        Some(path) => discover_graph_libraries(path)
            .map_err(|e| PlaybackServiceError::Registry(e.to_string()))?,
        None => Vec::new(),
    };
    aura_registry_with_libraries(&libraries)
        .map_err(|e| PlaybackServiceError::Registry(e.to_string()))
}

/// Parse graph JSON from the Lab host (bare Imp graph or tip-wrapped).
pub fn parse_graph_json(json: &str) -> Result<Graph, PlaybackServiceError> {
    // Prefer bare Graph; if that fails, strip a tip wrapper `{ "type": "...", "nodes", "edges" }`.
    match graph_from_json_str(json) {
        Ok(graph) => Ok(graph),
        Err(first) => {
            let value: serde_json::Value = serde_json::from_str(json)
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
    let node_id = find_output_node(&graph)
        .map_err(|e| PlaybackServiceError::Output(e.to_string()))?;
    let registry = playback_registry(neighbor_hint)?;
    let graph_dir = neighbor_hint.and_then(Path::parent);
    let empty_bindings = BTreeMap::new();
    let sampler = translate_graph_to_sampler_with_options(
        &graph,
        &registry,
        SAMPLE_RATE,
        TranslateOptions {
            score_bindings: &empty_bindings,
            graph_dir,
        },
    )
    .map_err(|e| PlaybackServiceError::Translate(e.to_string()))?;
    Ok((node_id, sampler))
}

/// Start (or restart) playback of the given graph JSON.
pub fn start_playback(
    graph_json: &str,
    neighbor_hint: Option<&Path>,
) -> Result<(), PlaybackServiceError> {
    let (_node_id, sampler) = prepare_graph_playback(graph_json, neighbor_hint)?;
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    engine
        .start(Arc::from(sampler), SAMPLE_RATE)
        .map_err(|e| PlaybackServiceError::Playback(e.to_string()))
}

/// Stop playback if running.
pub fn stop_playback() -> Result<(), PlaybackServiceError> {
    let mut engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    engine.stop();
    Ok(())
}

/// Toggle: stop if playing, otherwise start.
pub fn toggle_playback(
    graph_json: &str,
    neighbor_hint: Option<&Path>,
) -> Result<(), PlaybackServiceError> {
    let playing = {
        let engine = state()
            .engine
            .lock()
            .map_err(|_| PlaybackServiceError::Poisoned)?;
        engine.is_playing()
    };
    if playing {
        stop_playback()
    } else {
        start_playback(graph_json, neighbor_hint)
    }
}

/// Whether CPAL playback is currently active.
pub fn is_playing() -> Result<bool, PlaybackServiceError> {
    let engine = state()
        .engine
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    Ok(engine.is_playing())
}

fn read_graph_and_path<'a>(
    graph_json: *const c_uchar,
    graph_len: usize,
    neighbor_path: *const c_char,
) -> Result<(String, Option<PathBuf>), PlaybackServiceError> {
    if graph_json.is_null() || graph_len == 0 {
        return Err(PlaybackServiceError::GraphJson(
            "graph JSON is required".into(),
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
    match read_graph_and_path(graph_json, graph_len, neighbor_path) {
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
    match read_graph_and_path(graph_json, graph_len, neighbor_path) {
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
    // Ensure process-lifetime state exists before publishing the vtable.
    let _ = state();
    let service = ImpLabServiceV0 {
        ctx: std::ptr::null_mut(),
        start: Some(service_start),
        stop: Some(service_stop),
        toggle: Some(service_toggle),
        is_playing: Some(service_is_playing),
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
    fn register_installs_playback_service() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = ExtensionHost::new(dir.path());
        let vtable = host_vtable(&mut host);
        let code = unsafe { imp_lab_extension_register(&vtable) };
        assert_eq!(code, 0);
        assert!(host.services().get("playback").is_some());
    }
}
