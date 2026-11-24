//! Aura's side of the CLAP host callbacks.

use clack_extensions::audio_ports::PluginAudioPorts;
use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl, PluginGui};
use clack_extensions::latency::PluginLatency;
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
    PluginParams,
};
use clack_extensions::render::PluginRender;
use clack_extensions::state::{HostState, HostStateImpl, PluginState};
use clack_extensions::tail::PluginTail;
use clack_host::prelude::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub(crate) struct AuraHost;

impl HostHandlers for AuraHost {
    type Shared<'a> = AuraShared;
    type MainThread<'a> = AuraMainThread;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &AuraShared) {
        builder
            .register::<HostLog>()
            .register::<HostParams>()
            .register::<HostState>()
            .register::<HostGui>();
    }
}

/// Aura's main-thread host handler: records what a plugin reports about its state, for an open
/// plugin editor to pick up. Processing instances ignore it (they are configured only from node
/// data).
#[derive(Default)]
pub(crate) struct AuraMainThread {
    state_dirty: Cell<bool>,
    values_changed: Cell<bool>,
}

impl AuraMainThread {
    /// Whether the plugin reported its state or parameter values changed since the last call.
    pub(crate) fn take_changed(&self) -> bool {
        self.state_dirty.take() | self.values_changed.take()
    }
}

impl MainThreadHandler<'_> for AuraMainThread {}

impl HostParamsImplMainThread for AuraMainThread {
    fn rescan(&self, flags: ParamRescanFlags) {
        if flags.intersects(ParamRescanFlags::VALUES | ParamRescanFlags::ALL) {
            self.values_changed.set(true);
        }
    }

    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostStateImpl for AuraMainThread {
    fn mark_dirty(&self) {
        self.state_dirty.set(true);
    }
}

/// The plugin extensions Aura uses, captured once while the plugin initializes.
#[derive(Clone, Copy, Default)]
pub(crate) struct PluginExtensions {
    pub(crate) audio_ports: Option<PluginAudioPorts>,
    pub(crate) latency: Option<PluginLatency>,
    pub(crate) tail: Option<PluginTail>,
    pub(crate) state: Option<PluginState>,
    pub(crate) render: Option<PluginRender>,
    pub(crate) params: Option<PluginParams>,
    pub(crate) gui: Option<PluginGui>,
}

#[derive(Default)]
pub(crate) struct AuraShared {
    extensions: OnceLock<PluginExtensions>,
    callback_requested: AtomicBool,
    flush_requested: AtomicBool,
    gui_closed: AtomicBool,
    resize_requested: Mutex<Option<GuiSize>>,
}

impl AuraShared {
    pub(crate) fn extensions(&self) -> PluginExtensions {
        self.extensions.get().copied().unwrap_or_default()
    }

    pub(crate) fn take_callback_request(&self) -> bool {
        self.callback_requested.swap(false, Ordering::AcqRel)
    }

    /// Whether the plugin asked for a parameter flush (how a GUI reports a parameter change
    /// while the plugin is not processing).
    pub(crate) fn take_flush_request(&self) -> bool {
        self.flush_requested.swap(false, Ordering::AcqRel)
    }

    /// Whether the plugin closed its floating GUI window.
    pub(crate) fn take_gui_closed(&self) -> bool {
        self.gui_closed.swap(false, Ordering::AcqRel)
    }

    /// A size the plugin asked its host window to take.
    pub(crate) fn take_resize_request(&self) -> Option<GuiSize> {
        self.resize_requested.lock().ok()?.take()
    }
}

impl<'a> SharedHandler<'a> for AuraShared {
    fn initializing(&self, instance: InitializingPluginHandle<'a>) {
        let _ = self.extensions.set(PluginExtensions {
            audio_ports: instance.get_extension(),
            latency: instance.get_extension(),
            tail: instance.get_extension(),
            state: instance.get_extension(),
            render: instance.get_extension(),
            params: instance.get_extension(),
            gui: instance.get_extension(),
        });
    }

    /// Aura processes every block it renders and re-creates instances per translation, so a
    /// restart request has nothing further to act on.
    fn request_restart(&self) {}

    /// Aura never puts plugins to sleep, so processing is always already requested.
    fn request_process(&self) {}

    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::Release);
    }
}

impl HostParamsImplShared for AuraShared {
    /// Processing instances flush on every block anyway; an editor instance (not processing)
    /// flushes on the main thread when asked.
    fn request_flush(&self) {
        self.flush_requested.store(true, Ordering::Release);
    }
}

impl HostGuiImpl for AuraShared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        let mut requested = self
            .resize_requested
            .lock()
            .map_err(|_| HostError::Message("resize request lock poisoned"))?;
        *requested = Some(new_size);
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.gui_closed.store(true, Ordering::Release);
    }
}

impl HostLogImpl for AuraShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        eprintln!("aura: clap [{severity}] {message}");
    }
}
