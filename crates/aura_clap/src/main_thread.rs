//! The one CLAP "main thread" every plugin instance in the process shares.
//!
//! CLAP splits each plugin into a main-thread object (instantiate, state, activate, GUI,
//! callbacks) and an audio-thread processor. Many plugin frameworks (JUCE among them) assume a
//! single main thread per process, so Aura runs every main-thread call for every instance on
//! one dedicated thread. Instances live there for their whole life; only the `Send` audio
//! processor leaves it.

use crate::host::AuraHost;
use aura_plugin_host::PluginHostError;
use clack_host::prelude::PluginInstance;
use std::collections::HashMap;
use std::sync::mpsc::{channel, sync_channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// How often the main thread services plugins' `request_callback`s when otherwise idle.
const CALLBACK_POLL: Duration = Duration::from_millis(10);

pub(crate) type InstanceId = u64;

/// State owned by the main thread.
#[derive(Default)]
pub(crate) struct MainState {
    pub(crate) instances: HashMap<InstanceId, PluginInstance<AuraHost>>,
    next_id: InstanceId,
}

impl MainState {
    pub(crate) fn insert(&mut self, instance: PluginInstance<AuraHost>) -> InstanceId {
        let id = self.next_id;
        self.next_id += 1;
        self.instances.insert(id, instance);
        id
    }

    fn service_callbacks(&mut self) {
        for instance in self.instances.values_mut() {
            if instance.access_shared_handler(|shared| shared.take_callback_request()) {
                instance.call_on_main_thread_callback();
            }
        }
    }
}

type Job = Box<dyn FnOnce(&mut MainState) + Send>;

fn jobs() -> &'static Mutex<Sender<Job>> {
    static JOBS: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (sender, receiver) = channel::<Job>();
        thread::Builder::new()
            .name("aura-clap-main".into())
            .spawn(move || {
                let mut state = MainState::default();
                loop {
                    match receiver.recv_timeout(CALLBACK_POLL) {
                        Ok(job) => job(&mut state),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    state.service_callbacks();
                }
            })
            .expect("spawn the CLAP main thread");
        Mutex::new(sender)
    })
}

/// Runs `job` on the CLAP main thread and waits for its result.
///
/// Must not be called from the main thread itself (it would wait on itself).
pub(crate) fn run<R: Send + 'static>(
    job: impl FnOnce(&mut MainState) -> R + Send + 'static,
) -> Result<R, PluginHostError> {
    let gone = || PluginHostError::Instantiate {
        id: String::new(),
        reason: "the CLAP main thread has stopped".into(),
    };
    let (result_sender, result) = sync_channel(1);
    let job: Job = Box::new(move |state| {
        let _ = result_sender.send(job(state));
    });
    jobs()
        .lock()
        .map_err(|_| gone())?
        .send(job)
        .map_err(|_| gone())?;
    result.recv().map_err(|_| gone())
}
