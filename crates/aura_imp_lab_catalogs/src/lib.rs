//! Imp Lab `imp-catalog` extension: register Aura Imp libraries into the Lab host.

use aura_imp::{dsp, envelope, imp_file_from_json_str, music, plugin, time, types, ImpFile, NodeLibrary};
use imp_lab_extensions::{host_add_json, ImpLabHostV1};
use std::ffi::CStr;
use std::fs;
use std::os::raw::c_char;

fn push_node_library(host: *const ImpLabHostV1, library: NodeLibrary) -> i32 {
    let json = match serde_json::to_string(&library) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    unsafe {
        let h = &*host;
        host_add_json(h.add_node_library_json, h.ctx, &json)
    }
}

fn push_imp_file(host: *const ImpLabHostV1, file: ImpFile) -> i32 {
    let json = match serde_json::to_string(&file) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    unsafe {
        let h = &*host;
        host_add_json(h.add_imp_file_json, h.ctx, &json)
    }
}

fn resolve_pack_file(host: *const ImpLabHostV1, relative: &str) -> Option<std::path::PathBuf> {
    unsafe {
        let h = &*host;
        let resolve = h.resolve_pack_path?;
        let mut buf = vec![0u8; 4096];
        let rel_c = std::ffi::CString::new(relative).ok()?;
        let code = resolve(
            h.ctx,
            rel_c.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        );
        if code != 0 {
            return None;
        }
        let path = CStr::from_ptr(buf.as_ptr() as *const c_char)
            .to_str()
            .ok()?;
        Some(std::path::PathBuf::from(path))
    }
}

fn try_load_instruments(host: *const ImpLabHostV1) -> i32 {
    const CANDIDATES: &[&str] = &[
        "catalogs/instruments.json",
        "instruments.json",
        "demos/instruments.json",
    ];
    for rel in CANDIDATES {
        let Some(path) = resolve_pack_file(host, rel) else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let file = match imp_file_from_json_str(&text) {
            Ok(file) => file,
            Err(_) => continue,
        };
        return push_imp_file(host, file);
    }
    0
}

/// Lab extension entrypoint.
///
/// # Safety
///
/// `host` must be a valid pointer to an [`ImpLabHostV1`] provided by Imp Lab for the
/// duration of this call. Callbacks on the table must be safe to invoke as documented
/// by `imp_lab_extensions`.
#[no_mangle]
pub unsafe extern "C" fn imp_lab_extension_register(host: *const ImpLabHostV1) -> i32 {
    if host.is_null() {
        return -1;
    }

    // Aura's named unions (`Audio`, `Loopable`) travel as a types-only node library.
    if push_node_library(host, types::aura_types_library()) != 0 {
        return -1;
    }
    if push_node_library(host, time::time_node_library()) != 0 {
        return -1;
    }
    if push_node_library(host, dsp::dsp_node_library()) != 0 {
        return -1;
    }
    if push_node_library(host, envelope::envelope_node_library()) != 0 {
        return -1;
    }
    if push_node_library(host, music::music_node_library()) != 0 {
        return -1;
    }
    if push_node_library(host, plugin::plugin_node_library()) != 0 {
        return -1;
    }
    // Optional demo instruments; failure to find is not fatal.
    let _ = try_load_instruments(host);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use imp_lab_extensions::{host_vtable, ExtensionHost};

    #[test]
    fn register_pushes_aura_libraries() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = ExtensionHost::new(dir.path());
        let vtable = host_vtable(&mut host);
        let code = unsafe { imp_lab_extension_register(&vtable) };
        assert_eq!(code, 0);
        assert!(host.node_libraries().len() >= 5);
        let audio = host
            .node_libraries()
            .iter()
            .flat_map(|lib| lib["types"].as_array().into_iter().flatten())
            .find(|ty| ty["id"] == "Audio");
        assert_eq!(
            audio.map(|ty| ty["type"].clone()),
            Some(serde_json::json!({ "union": [{ "id": "audio_mono" }, { "id": "audio_stereo" }] }))
        );
        let sine = host
            .node_libraries()
            .iter()
            .flat_map(|lib| lib["definitions"].as_array().into_iter().flatten())
            .find(|def| def["id"] == "sine");
        assert!(sine.is_some());
        assert!(sine.unwrap().get("typeParams").is_some() || sine.unwrap().get("inputs").is_some());
    }
}
