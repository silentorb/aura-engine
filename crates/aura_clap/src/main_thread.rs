//! The one CLAP "main thread" every plugin instance in the process shares.
//!
//! CLAP splits each plugin into a main-thread object (instantiate, state, activate, GUI,
//! callbacks) and an audio-thread processor. Many plugin frameworks (JUCE among them) assume a
//! single main thread per process, so Aura runs every main-thread call for every instance on
//! one dedicated thread. Instances live there for their whole life; only the `Send` audio
//! processor leaves it.
//!
//! Where a display is available the thread runs a windowing event loop (which also pumps the
//! platform's window messages), so plugin GUIs can be embedded in host windows. Elsewhere
//! (headless CI, containers) it runs a plain job loop and only floating plugin GUIs can open.

use crate::editor::Editors;
use crate::host::AuraHost;
use crate::windowing::WindowSystem;
use aura_plugin_host::PluginHostError;
use clack_host::prelude::PluginInstance;
use std::collections::HashMap;
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// How often the main thread services plugins' callbacks and open editors when otherwise idle.
pub(crate) const SERVICE_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) type InstanceId = u64;

/// State owned by the main thread.
#[derive(Default)]
pub(crate) struct MainState {
    pub(crate) instances: HashMap<InstanceId, PluginInstance<AuraHost>>,
    pub(crate) editors: Editors,
    next_id: InstanceId,
}

impl MainState {
    pub(crate) fn insert(&mut self, instance: PluginInstance<AuraHost>) -> InstanceId {
        let id = self.next_id;
        self.next_id += 1;
        self.instances.insert(id, instance);
        id
    }

    /// Plugins' main-thread callbacks, then open editors.
    pub(crate) fn service(&mut self, windows: Option<&WindowSystem<'_>>) {
        for instance in self.instances.values_mut() {
            if instance.access_shared_handler(|shared| shared.take_callback_request()) {
                instance.call_on_main_thread_callback();
            }
        }
        self.editors.service(&mut self.instances, windows);
    }
}

/// Work for the main thread. `windows` is the windowing system when one is running.
pub(crate) type Job = Box<dyn FnOnce(&mut MainState, Option<&WindowSystem<'_>>) + Send>;

struct MainThread {
    jobs: Sender<Job>,
    wake: Option<crate::windowing::Waker>,
}

fn main_thread() -> &'static Mutex<MainThread> {
    static MAIN: OnceLock<Mutex<MainThread>> = OnceLock::new();
    MAIN.get_or_init(|| {
        let (jobs, receiver) = channel::<Job>();
        let (ready, waker) = sync_channel(1);
        thread::Builder::new()
            .name("aura-clap-main".into())
            .spawn(move || crate::windowing::run(receiver, ready))
            .expect("spawn the CLAP main thread");
        // `None` until the thread says whether a windowing loop started.
        let wake = waker.recv().ok().flatten();
        Mutex::new(MainThread { jobs, wake })
    })
}

/// The loop used without a windowing system: run jobs, service plugins.
pub(crate) fn headless_loop(receiver: Receiver<Job>) {
    let mut state = MainState::default();
    loop {
        match receiver.recv_timeout(SERVICE_INTERVAL) {
            Ok(job) => job(&mut state, None),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        state.service(None);
    }
}

/// Runs `job` on the CLAP main thread and waits for its result.
///
/// Must not be called from the main thread itself (it would wait on itself).
pub(crate) fn run_with_windows<R: Send + 'static>(
    job: impl FnOnce(&mut MainState, Option<&WindowSystem<'_>>) -> R + Send + 'static,
) -> Result<R, PluginHostError> {
    let gone = || PluginHostError::Instantiate {
        id: String::new(),
        reason: "the CLAP main thread has stopped".into(),
    };
    let (result_sender, result) = sync_channel(1);
    let job: Job = Box::new(move |state, windows| {
        let _ = result_sender.send(job(state, windows));
    });
    {
        let main = main_thread().lock().map_err(|_| gone())?;
        main.jobs.send(job).map_err(|_| gone())?;
        if let Some(wake) = &main.wake {
            wake.wake();
        }
    }
    result.recv().map_err(|_| gone())
}

/// Runs `job` on the CLAP main thread and waits for its result.
pub(crate) fn run<R: Send + 'static>(
    job: impl FnOnce(&mut MainState) -> R + Send + 'static,
) -> Result<R, PluginHostError> {
    run_with_windows(move |state, _| job(state))
}
