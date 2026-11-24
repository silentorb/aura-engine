//! CLAP backend for Aura's plugin host (`aura_plugin_host`).
//!
//! [`ClapHost`] finds `.clap` bundles, lists the plugins they contain, and resolves plugin ids
//! to [`ClapFactory`]s whose instances implement [`aura_plugin_host::Processor`]. Every
//! instance's main-thread half runs on one shared CLAP main thread.
//!
//! Plugins run in-process: a plugin is native code, and a crashing plugin takes the host with
//! it. Design: `docs/design/plugin-hosting.md`.

mod editor;
mod host;
mod main_thread;
mod processor;
mod windowing;

pub use editor::{EditorEvent, SAVE_DELAY};
pub use processor::{ClapFactory, ClapProcessor};

use aura_plugin_host::{PluginHostError, PluginResolver, PluginSettings, ProcessorFactory};
use clack_host::prelude::PluginEntry;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A plugin found in a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub features: Vec<String>,
    pub bundle: PathBuf,
}

/// A bundle that could not be loaded while scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFailure {
    pub bundle: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub plugins: Vec<PluginInfo>,
    pub failures: Vec<ScanFailure>,
}

struct Bundle {
    entry: PluginEntry,
    plugins: Vec<PluginInfo>,
}

/// Finds CLAP plugins and resolves them by id.
///
/// Bundles load lazily, on the first [`ClapHost::discover`] or resolve, and stay loaded for the
/// host's life. Entries added with [`ClapHost::add_entry`] (in-process plugins) are searched
/// first.
pub struct ClapHost {
    search_paths: Vec<PathBuf>,
    scanned: Mutex<Option<(Vec<Bundle>, Vec<ScanFailure>)>>,
    extra: Mutex<Vec<Bundle>>,
}

impl ClapHost {
    /// A host searching `search_paths` (directories, scanned recursively, or `.clap` files).
    pub fn new(search_paths: Vec<PathBuf>) -> Self {
        Self {
            search_paths,
            scanned: Mutex::new(None),
            extra: Mutex::new(Vec::new()),
        }
    }

    /// A host searching `extra` and then the platform's standard CLAP locations.
    pub fn with_standard_paths(extra: Vec<PathBuf>) -> Self {
        let mut paths = extra;
        paths.extend(standard_paths());
        Self::new(paths)
    }

    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search_paths
    }

    /// Adds an already loaded entry (for example an in-process Rust plugin under test).
    pub fn add_entry(&self, entry: PluginEntry, label: impl Into<PathBuf>) {
        let label = label.into();
        let plugins = describe(&entry, &label);
        let mut extra = self.extra.lock().unwrap_or_else(|p| p.into_inner());
        extra.push(Bundle { entry, plugins });
    }

    /// Every plugin in the added entries and search paths, plus bundles that failed to load.
    pub fn discover(&self) -> Discovery {
        let mut discovery = Discovery::default();
        {
            let extra = self.extra.lock().unwrap_or_else(|p| p.into_inner());
            for bundle in extra.iter() {
                discovery.plugins.extend(bundle.plugins.iter().cloned());
            }
        }
        self.with_scanned(|bundles, failures| {
            for bundle in bundles {
                discovery.plugins.extend(bundle.plugins.iter().cloned());
            }
            discovery.failures.extend(failures.iter().cloned());
        });
        discovery
    }

    fn with_scanned<R>(&self, f: impl FnOnce(&[Bundle], &[ScanFailure]) -> R) -> R {
        let mut guard = self.scanned.lock().unwrap_or_else(|p| p.into_inner());
        let (bundles, failures) = guard.get_or_insert_with(|| scan(&self.search_paths));
        f(bundles, failures)
    }

    fn find_entry(&self, plugin_id: &str) -> Option<PluginEntry> {
        let has = |bundle: &Bundle| bundle.plugins.iter().any(|p| p.id == plugin_id);
        {
            let extra = self.extra.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(bundle) = extra.iter().find(|b| has(b)) {
                return Some(bundle.entry.clone());
            }
        }
        self.with_scanned(|bundles, _| bundles.iter().find(|b| has(b)).map(|b| b.entry.clone()))
    }
}

impl ClapHost {
    /// Opens `plugin_id`'s own GUI on a fresh instance configured with `settings`, titled
    /// `title`. `listener` receives the plugin's saved state as the user edits it, and
    /// [`EditorEvent::Closed`] when the editor closes. Runs on the CLAP main thread.
    pub fn open_editor(
        &self,
        plugin_id: &str,
        settings: &PluginSettings,
        title: &str,
        listener: impl FnMut(EditorEvent) + Send + 'static,
    ) -> Result<EditorHandle, PluginHostError> {
        let entry = self
            .find_entry(plugin_id)
            .ok_or_else(|| PluginHostError::NotFound {
                id: plugin_id.into(),
            })?;
        let id = plugin_id.to_string();
        let settings = settings.clone();
        let title = title.to_string();
        let host = processor::host_info()?;
        let editor = main_thread::run_with_windows(move |main, windows| {
            let mut instance = processor::instantiate(&entry, &id, &host)?;
            processor::configure_inactive(&id, &mut instance, &settings)?;
            let instance_id = main.insert(instance);
            let main_thread::MainState {
                instances, editors, ..
            } = main;
            editors.open(&id, instance_id, instances, windows, &title, Box::new(listener))
        })??;
        Ok(EditorHandle { id: editor })
    }
}

/// An open plugin editor. It stays open until the user closes it or [`EditorHandle::close`]
/// is called; dropping the handle does not close it.
#[derive(Debug)]
pub struct EditorHandle {
    id: editor::EditorId,
}

impl EditorHandle {
    pub fn is_open(&self) -> bool {
        let id = self.id;
        main_thread::run(move |main| main.editors.is_open(id)).unwrap_or(false)
    }

    /// Brings the editor's window to the front (an embedded GUI only).
    pub fn focus(&self) {
        let id = self.id;
        let _ = main_thread::run(move |main| main.editors.focus(id));
    }

    /// Closes the editor, saving a pending change first.
    pub fn close(&self) {
        let id = self.id;
        let _ = main_thread::run(move |main| {
            let main_thread::MainState {
                instances, editors, ..
            } = main;
            editors.close(id, instances);
        });
    }
}

impl PluginResolver for ClapHost {
    fn resolve(
        &self,
        plugin_id: &str,
        settings: &PluginSettings,
    ) -> Result<Arc<dyn ProcessorFactory>, PluginHostError> {
        let entry = self
            .find_entry(plugin_id)
            .ok_or_else(|| PluginHostError::NotFound {
                id: plugin_id.into(),
            })?;
        Ok(Arc::new(ClapFactory::new(
            entry,
            plugin_id.into(),
            settings.clone(),
        )))
    }
}

/// The platform's standard CLAP locations, after any `CLAP_PATH` entries.
pub fn standard_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::env::var_os("CLAP_PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    if cfg!(target_os = "windows") {
        if let Some(common) = std::env::var_os("COMMONPROGRAMFILES") {
            paths.push(PathBuf::from(common).join("CLAP"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join("Common")
                    .join("CLAP"),
            );
        }
    } else if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/CLAP"));
        }
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
    } else {
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".clap"));
        }
        paths.push(PathBuf::from("/usr/lib/clap"));
    }
    paths
}

fn scan(search_paths: &[PathBuf]) -> (Vec<Bundle>, Vec<ScanFailure>) {
    let mut files = Vec::new();
    for path in search_paths {
        collect_bundles(path, &mut files);
    }
    files.sort();
    files.dedup();
    let mut bundles = Vec::new();
    let mut failures = Vec::new();
    for file in files {
        // SAFETY: loading a CLAP bundle runs its native initialization code. That is what
        // hosting a plugin means; the user chose to install it in a CLAP search path.
        match unsafe { PluginEntry::load(&file) } {
            Ok(entry) => {
                let plugins = describe(&entry, &file);
                bundles.push(Bundle { entry, plugins });
            }
            Err(error) => failures.push(ScanFailure {
                bundle: file,
                reason: error.to_string(),
            }),
        }
    }
    (bundles, failures)
}

fn is_bundle(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("clap"))
}

fn collect_bundles(path: &Path, out: &mut Vec<PathBuf>) {
    if is_bundle(path) {
        out.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_bundles(&entry.path(), out);
    }
}

fn describe(entry: &PluginEntry, bundle: &Path) -> Vec<PluginInfo> {
    let text = |value: Option<&std::ffi::CStr>| {
        value
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let Some(factory) = entry.get_plugin_factory() else {
        return Vec::new();
    };
    factory
        .plugin_descriptors()
        .filter_map(|descriptor| {
            let id = descriptor.id()?.to_string_lossy().into_owned();
            Some(PluginInfo {
                id,
                name: text(descriptor.name()),
                vendor: text(descriptor.vendor()),
                version: text(descriptor.version()),
                features: descriptor
                    .features()
                    .map(|f| f.to_string_lossy().into_owned())
                    .collect(),
                bundle: bundle.to_path_buf(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
