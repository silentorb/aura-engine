//! Aura's side of the CLAP host callbacks.

use clack_extensions::audio_ports::PluginAudioPorts;
use clack_extensions::latency::PluginLatency;
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::render::PluginRender;
use clack_extensions::state::PluginState;
use clack_extensions::tail::PluginTail;
use clack_host::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

pub(crate) struct AuraHost;

impl HostHandlers for AuraHost {
    type Shared<'a> = AuraShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &AuraShared) {
        builder.register::<HostLog>();
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
}

#[derive(Default)]
pub(crate) struct AuraShared {
    extensions: OnceLock<PluginExtensions>,
    callback_requested: AtomicBool,
}

impl AuraShared {
    pub(crate) fn extensions(&self) -> PluginExtensions {
        self.extensions.get().copied().unwrap_or_default()
    }

    pub(crate) fn take_callback_request(&self) -> bool {
        self.callback_requested.swap(false, Ordering::AcqRel)
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

impl HostLogImpl for AuraShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        eprintln!("aura: clap [{severity}] {message}");
    }
}
