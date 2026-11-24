//! Plugin editors: a plugin's own GUI, opened for one node, reporting its saved state as the
//! user edits it.
//!
//! An editor owns its own (unactivated) instance on the CLAP main thread, configured from the
//! node's settings. It embeds the plugin GUI in a host window where a window system is running
//! and the plugin supports embedding, and otherwise asks the plugin for a floating window. When
//! the plugin reports a change (state marked dirty, a parameter flush requested, or values
//! rescanned), the editor saves the plugin's state [`SAVE_DELAY`] after the last change and
//! hands it to the editor's listener; closing saves any pending change first.

use crate::host::{AuraHost, AuraMainThread, AuraShared};
use crate::main_thread::InstanceId;
use crate::windowing::{HostWindow, WindowKey, WindowSystem};
use aura_plugin_host::PluginHostError;
use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize, PluginGui};
use clack_extensions::params::PluginParams;
use clack_extensions::state::PluginState;
use clack_host::events::io::{EventBuffer, InputEvents, OutputEvents};
use clack_host::prelude::PluginInstance;
use std::collections::HashMap;
use std::ffi::CString;
use std::time::{Duration, Instant};

/// How long after the last reported change an editor saves the plugin's state.
pub const SAVE_DELAY: Duration = Duration::from_millis(300);
const DEFAULT_SIZE: GuiSize = GuiSize {
    width: 640,
    height: 480,
};

/// What an editor reports to its listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorEvent {
    /// The plugin's state after the user's edits (opaque, plugin-owned).
    State(Vec<u8>),
    /// The editor closed (by the user, the plugin, or [`crate::EditorHandle::close`]).
    Closed,
}

pub(crate) type Listener = Box<dyn FnMut(EditorEvent) + Send>;
pub(crate) type EditorId = u64;

struct Editor {
    instance: InstanceId,
    gui: PluginGui,
    state: Option<PluginState>,
    params: Option<PluginParams>,
    /// The host window the GUI is embedded in; `None` for a floating GUI.
    window: Option<HostWindow>,
    listener: Listener,
    changed_at: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct Editors {
    open: HashMap<EditorId, Editor>,
    next_id: EditorId,
}

fn gui_error(id: &str, reason: impl ToString) -> PluginHostError {
    PluginHostError::Gui {
        id: id.into(),
        reason: reason.to_string(),
    }
}

impl Editors {
    /// Opens `instance`'s GUI (taking ownership of the instance).
    pub(crate) fn open(
        &mut self,
        plugin_id: &str,
        instance_id: InstanceId,
        instances: &mut HashMap<InstanceId, PluginInstance<AuraHost>>,
        windows: Option<&WindowSystem<'_>>,
        title: &str,
        listener: Listener,
    ) -> Result<EditorId, PluginHostError> {
        let opened = self.create_gui(plugin_id, instance_id, instances, windows, title);
        let (gui, window) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                instances.remove(&instance_id);
                return Err(error);
            }
        };
        let extensions = instances[&instance_id].access_shared_handler(AuraShared::extensions);
        let id = self.next_id;
        self.next_id += 1;
        self.open.insert(
            id,
            Editor {
                instance: instance_id,
                gui,
                state: extensions.state,
                params: extensions.params,
                window,
                listener,
                changed_at: None,
            },
        );
        Ok(id)
    }

    fn create_gui(
        &self,
        plugin_id: &str,
        instance_id: InstanceId,
        instances: &mut HashMap<InstanceId, PluginInstance<AuraHost>>,
        windows: Option<&WindowSystem<'_>>,
        title: &str,
    ) -> Result<(PluginGui, Option<HostWindow>), PluginHostError> {
        let instance = instances
            .get_mut(&instance_id)
            .ok_or_else(|| gui_error(plugin_id, "instance is gone"))?;
        let gui = instance
            .access_shared_handler(AuraShared::extensions)
            .gui
            .ok_or_else(|| PluginHostError::NoGui {
                id: plugin_id.into(),
            })?;
        let api = GuiApiType::default_for_current_platform()
            .ok_or_else(|| gui_error(plugin_id, "no GUI API on this platform"))?;
        let plugin = instance.plugin_handle();
        let embedded = GuiConfiguration {
            api_type: api,
            is_floating: false,
        };
        let floating = GuiConfiguration {
            api_type: api,
            is_floating: true,
        };
        if let Some(windows) = windows.filter(|_| gui.is_api_supported(&plugin, embedded)) {
            gui.create(&plugin, embedded)
                .map_err(|e| gui_error(plugin_id, e))?;
            let size = gui.get_size(&plugin).unwrap_or(DEFAULT_SIZE);
            let window = windows
                .create_window(title, size, gui.can_resize(&plugin))
                .map_err(|e| gui_error(plugin_id, e));
            let attached = window.and_then(|window| {
                let parent = window
                    .clap_parent()
                    .ok_or_else(|| gui_error(plugin_id, "window has no CLAP handle"))?;
                // SAFETY: the window outlives the GUI: the editor destroys the GUI before it
                // drops the window.
                unsafe { gui.set_parent(&plugin, parent) }
                    .map_err(|e| gui_error(plugin_id, e))?;
                gui.show(&plugin).map_err(|e| gui_error(plugin_id, e))?;
                Ok(window)
            });
            return match attached {
                Ok(window) => Ok((gui, Some(window))),
                Err(error) => {
                    gui.destroy(&plugin);
                    Err(error)
                }
            };
        }
        if gui.is_api_supported(&plugin, floating) {
            gui.create(&plugin, floating)
                .map_err(|e| gui_error(plugin_id, e))?;
            if let Ok(title) = CString::new(title) {
                gui.suggest_title(&plugin, &title);
            }
            if let Err(error) = gui.show(&plugin) {
                gui.destroy(&plugin);
                return Err(gui_error(plugin_id, error));
            }
            return Ok((gui, None));
        }
        Err(gui_error(
            plugin_id,
            if windows.is_some() {
                "the plugin supports neither an embedded nor a floating GUI here"
            } else {
                "no window system is running and the plugin has no floating GUI"
            },
        ))
    }

    pub(crate) fn is_open(&self, id: EditorId) -> bool {
        self.open.contains_key(&id)
    }

    pub(crate) fn focus(&self, id: EditorId) {
        if let Some(window) = self.open.get(&id).and_then(|e| e.window.as_ref()) {
            window.focus();
        }
    }

    /// Closes an editor, saving any pending change first.
    pub(crate) fn close(
        &mut self,
        id: EditorId,
        instances: &mut HashMap<InstanceId, PluginInstance<AuraHost>>,
    ) {
        let Some(mut editor) = self.open.remove(&id) else {
            return;
        };
        if let Some(instance) = instances.get_mut(&editor.instance) {
            let pending = editor.changed_at.is_some()
                || instance.access_handler(AuraMainThread::take_changed);
            if pending {
                save(&mut editor, instance);
            }
            editor.gui.destroy(&instance.plugin_handle());
        }
        // The GUI is destroyed before its window goes.
        drop(editor.window.take());
        instances.remove(&editor.instance);
        (editor.listener)(EditorEvent::Closed);
    }

    pub(crate) fn close_window(
        &mut self,
        key: WindowKey,
        instances: &mut HashMap<InstanceId, PluginInstance<AuraHost>>,
    ) {
        if let Some(id) = self.by_window(key) {
            self.close(id, instances);
        }
    }

    pub(crate) fn window_resized(
        &mut self,
        key: WindowKey,
        size: GuiSize,
        instances: &mut HashMap<InstanceId, PluginInstance<AuraHost>>,
    ) {
        let Some(editor) = self.by_window(key).and_then(|id| self.open.get(&id)) else {
            return;
        };
        if let Some(instance) = instances.get_mut(&editor.instance) {
            let plugin = instance.plugin_handle();
            if editor.gui.can_resize(&plugin) {
                let size = editor.gui.adjust_size(&plugin, size).unwrap_or(size);
                let _ = editor.gui.set_size(&plugin, size);
            }
        }
    }

    fn by_window(&self, key: WindowKey) -> Option<EditorId> {
        self.open
            .iter()
            .find(|(_, e)| e.window.as_ref().is_some_and(|w| w.key() == key))
            .map(|(id, _)| *id)
    }

    /// Picks up reported changes, saves settled ones, applies resize requests, and closes
    /// editors whose floating GUI the plugin closed.
    pub(crate) fn service(
        &mut self,
        instances: &mut HashMap<InstanceId, PluginInstance<AuraHost>>,
        _windows: Option<&WindowSystem<'_>>,
    ) {
        let mut closed = Vec::new();
        let now = Instant::now();
        for (id, editor) in &mut self.open {
            let Some(instance) = instances.get_mut(&editor.instance) else {
                closed.push(*id);
                continue;
            };
            let shared_closed = instance.access_shared_handler(AuraShared::take_gui_closed);
            let mut changed = instance.access_handler(AuraMainThread::take_changed);
            if instance.access_shared_handler(AuraShared::take_flush_request) {
                changed |= flush_params(editor.params, instance);
            }
            if changed {
                editor.changed_at = Some(now);
            }
            if let Some(size) = instance.access_shared_handler(AuraShared::take_resize_request) {
                if let Some(window) = &editor.window {
                    window.set_size(size);
                }
            }
            if editor
                .changed_at
                .is_some_and(|at| now.duration_since(at) >= SAVE_DELAY)
            {
                save(editor, instance);
            }
            if shared_closed {
                closed.push(*id);
            }
        }
        for id in closed {
            self.close(id, instances);
        }
    }
}

/// Flushes parameter changes a GUI made while the plugin is not processing. Returns whether the
/// plugin reported any.
fn flush_params(params: Option<PluginParams>, instance: &mut PluginInstance<AuraHost>) -> bool {
    let (Some(params), Some(mut plugin)) = (params, instance.inactive_plugin_handle()) else {
        return false;
    };
    let mut output = EventBuffer::new();
    params.flush(
        &mut plugin,
        &InputEvents::empty(),
        &mut OutputEvents::from_buffer(&mut output),
    );
    // The flush was requested because something changed, even if no event says what.
    true
}

fn save(editor: &mut Editor, instance: &mut PluginInstance<AuraHost>) {
    editor.changed_at = None;
    let Some(state) = editor.state else {
        return;
    };
    let mut bytes = Vec::new();
    match state.save(&instance.plugin_handle(), &mut bytes) {
        Ok(()) => (editor.listener)(EditorEvent::State(bytes)),
        Err(error) => eprintln!("aura: could not save plugin state: {error}"),
    }
}
