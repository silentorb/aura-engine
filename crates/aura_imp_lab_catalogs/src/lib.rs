//! Imp Lab `imp-catalog` extension: register Aura Imp libraries into the Lab host.

use aura_imp::{constraints, dsp, envelope, music, time, GraphLibrary, NodeLibrary};
use imp_core_types::TypeConstraintLibrary;
use imp_lab_extensions::{host_add_json, ImpLabHostV0};
use std::ffi::CStr;
use std::fs;
use std::os::raw::c_char;

fn push_node_library(host: *const ImpLabHostV0, library: NodeLibrary) -> i32 {
    let json = match serde_json::to_string(&library) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    unsafe {
        let h = &*host;
        host_add_json(h.add_node_library_json, h.ctx, &json)
    }
}

fn push_constraints(host: *const ImpLabHostV0, library: TypeConstraintLibrary) -> i32 {
    let json = match serde_json::to_string(&library) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    unsafe {
        let h = &*host;
        host_add_json(h.add_type_constraint_library_json, h.ctx, &json)
    }
}

fn push_graph_library(host: *const ImpLabHostV0, library: GraphLibrary) -> i32 {
    let json = match serde_json::to_string(&library) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    unsafe {
        let h = &*host;
        host_add_json(h.add_graph_library_json, h.ctx, &json)
    }
}

fn resolve_pack_file(host: *const ImpLabHostV0, relative: &str) -> Option<std::path::PathBuf> {
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

fn try_load_instruments(host: *const ImpLabHostV0) -> i32 {
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
        // Strip optional tip wrapper if present; GraphLibrary is the object itself or under type.
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let library: GraphLibrary = match serde_json::from_value(value) {
            Ok(lib) => lib,
            Err(_) => continue,
        };
        return push_graph_library(host, library);
    }
    0
}

/// Lab extension entrypoint.
///
/// # Safety
///
/// `host` must be a valid pointer to an [`ImpLabHostV0`] provided by Imp Lab for the
/// duration of this call. Callbacks on the table must be safe to invoke as documented
/// by `imp_lab_extensions`.
#[no_mangle]
pub unsafe extern "C" fn imp_lab_extension_register(host: *const ImpLabHostV0) -> i32 {
    if host.is_null() {
        return -1;
    }

    if push_constraints(host, constraints::aura_type_constraint_library()) != 0 {
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
        assert!(host.node_libraries().len() >= 4);
        assert_eq!(host.type_constraint_libraries().len(), 1);
        let sine = host
            .node_libraries()
            .iter()
            .flat_map(|lib| lib["definitions"].as_array().into_iter().flatten())
            .find(|def| def["id"] == "sine");
        assert!(sine.is_some());
        assert!(sine.unwrap().get("typeParams").is_some() || sine.unwrap().get("inputs").is_some());
    }
}
