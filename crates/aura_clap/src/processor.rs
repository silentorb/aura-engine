//! CLAP plugin instances as Aura [`Processor`]s.

use crate::host::{AuraHost, AuraShared, PluginExtensions};
use crate::main_thread::{self, InstanceId};
use aura_plugin_host::{
    ChannelLayout, Event, PluginHostError, Processor, ProcessorConfig, ProcessorFactory,
    RenderMode, Tail,
};
use clack_extensions::audio_ports::{AudioPortFlags, AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::render::RenderMode as ClapRenderMode;
use clack_extensions::tail::TailLength;
use clack_host::events::event_types::{NoteOffEvent, NoteOnEvent};
use clack_host::prelude::*;
use std::ffi::CString;
use std::io::Cursor;

/// Most events Aura sends in one block before the buffer would grow.
const EVENT_CAPACITY: usize = 512;

/// Creates instances of one CLAP plugin from a loaded bundle, applying stored state.
pub struct ClapFactory {
    entry: PluginEntry,
    id: String,
    state: Option<Vec<u8>>,
}

impl ClapFactory {
    pub(crate) fn new(entry: PluginEntry, id: String, state: Option<Vec<u8>>) -> Self {
        Self { entry, id, state }
    }
}

fn host_info() -> Result<HostInfo, PluginHostError> {
    HostInfo::new(
        "Aura",
        "Silent Orb",
        "https://github.com/silentorb/aura-engine",
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| PluginHostError::Instantiate {
        id: String::new(),
        reason: e.to_string(),
    })
}

/// Channel counts of every audio port in one direction, plus the index of the main port.
#[derive(Debug, Clone, Default)]
struct PortLayout {
    channels: Vec<usize>,
    main: Option<usize>,
}

impl PortLayout {
    fn read(
        ports: Option<PluginAudioPorts>,
        plugin: &mut PluginMainThreadHandle,
        is_input: bool,
    ) -> Self {
        let Some(ports) = ports else {
            return Self::default();
        };
        let mut buffer = AudioPortInfoBuffer::new();
        let mut layout = Self::default();
        for index in 0..ports.count(plugin, is_input) {
            let Some(info) = ports.get(plugin, index, is_input, &mut buffer) else {
                layout.channels.push(0);
                continue;
            };
            if layout.main.is_none() && info.flags.contains(AudioPortFlags::IS_MAIN) {
                layout.main = Some(layout.channels.len());
            }
            layout.channels.push(info.channel_count as usize);
        }
        if layout.main.is_none() && !layout.channels.is_empty() {
            layout.main = Some(0);
        }
        layout
    }

    fn main_channels(&self) -> usize {
        self.main.map(|m| self.channels[m]).unwrap_or(0)
    }

    fn total_channels(&self) -> usize {
        self.channels.iter().sum()
    }
}

/// What the main thread hands back from creating an instance.
struct Created {
    instance: InstanceId,
    processor: StoppedPluginAudioProcessor<AuraHost>,
    extensions: PluginExtensions,
    inputs: PortLayout,
    outputs: PortLayout,
    latency: u32,
}

impl ProcessorFactory for ClapFactory {
    fn plugin_id(&self) -> &str {
        &self.id
    }

    fn create(&self, config: ProcessorConfig) -> Result<Box<dyn Processor>, PluginHostError> {
        let entry = self.entry.clone();
        let id = self.id.clone();
        let state = self.state.clone();
        let host = host_info()?;
        let created = main_thread::run(move |main| -> Result<Created, PluginHostError> {
            let instantiate = |reason: String| PluginHostError::Instantiate {
                id: id.clone(),
                reason,
            };
            let c_id = CString::new(id.as_str()).map_err(|e| instantiate(e.to_string()))?;
            let mut instance = PluginInstance::<AuraHost>::new(
                |_| AuraShared::default(),
                |_| (),
                &entry,
                &c_id,
                &host,
            )
            .map_err(|e| instantiate(e.to_string()))?;
            let extensions = instance.access_shared_handler(AuraShared::extensions);

            if let Some(state) = &state {
                let ext = extensions
                    .state
                    .ok_or_else(|| PluginHostError::StateLoad { id: id.clone() })?;
                ext.load(&instance.plugin_handle(), &mut Cursor::new(state))
                    .map_err(|_| PluginHostError::StateLoad { id: id.clone() })?;
            }
            if let Some(render) = extensions.render {
                let mode = match config.mode {
                    RenderMode::Offline => ClapRenderMode::Offline,
                    RenderMode::Realtime => ClapRenderMode::Realtime,
                };
                // A plugin may refuse a render mode; it then renders in its default mode,
                // which is still correct output.
                let _ = render.set(&instance.plugin_handle(), mode);
            }
            let inputs = PortLayout::read(extensions.audio_ports, &mut instance.plugin_handle(), true);
            let outputs =
                PortLayout::read(extensions.audio_ports, &mut instance.plugin_handle(), false);

            let processor = instance
                .activate(
                    |_, _| (),
                    PluginAudioConfiguration {
                        sample_rate: f64::from(config.sample_rate.get()),
                        min_frames_count: 1,
                        max_frames_count: config.max_block_frames,
                    },
                )
                .map_err(|e| PluginHostError::Activate {
                    id: id.clone(),
                    reason: e.to_string(),
                })?;
            let latency = extensions
                .latency
                .map(|latency| latency.get(&instance.plugin_handle()))
                .unwrap_or(0);
            Ok(Created {
                instance: main.insert(instance),
                processor,
                extensions,
                inputs,
                outputs,
                latency,
            })
        })??;

        let max = config.max_block_frames as usize;
        let buffers = |layout: &PortLayout| -> Vec<Vec<Vec<f32>>> {
            layout
                .channels
                .iter()
                .map(|&channels| vec![vec![0.0; max]; channels])
                .collect()
        };
        Ok(Box::new(ClapProcessor {
            id: self.id.clone(),
            instance: created.instance,
            processor: Some(created.processor.into()),
            extensions: created.extensions,
            in_bufs: buffers(&created.inputs),
            out_bufs: buffers(&created.outputs),
            in_ports: AudioPorts::with_capacity(
                created.inputs.total_channels(),
                created.inputs.channels.len(),
            ),
            out_ports: AudioPorts::with_capacity(
                created.outputs.total_channels(),
                created.outputs.channels.len(),
            ),
            inputs: created.inputs,
            outputs: created.outputs,
            latency: created.latency,
            events: EventBuffer::with_capacity(EVENT_CAPACITY),
            out_events: EventBuffer::with_capacity(EVENT_CAPACITY),
            steady_time: 0,
        }))
    }
}

/// An activated CLAP instance. Its main-thread half stays on the CLAP main thread; this is the
/// audio-thread half. Dropping it deactivates and destroys the instance there.
pub struct ClapProcessor {
    id: String,
    instance: InstanceId,
    processor: Option<PluginAudioProcessor<AuraHost>>,
    extensions: PluginExtensions,
    inputs: PortLayout,
    outputs: PortLayout,
    /// `[port][channel][frame]`
    in_bufs: Vec<Vec<Vec<f32>>>,
    out_bufs: Vec<Vec<Vec<f32>>>,
    in_ports: AudioPorts,
    out_ports: AudioPorts,
    latency: u32,
    events: EventBuffer,
    out_events: EventBuffer,
    steady_time: u64,
}

impl ClapProcessor {
    fn process_error(&self, reason: impl ToString) -> PluginHostError {
        PluginHostError::Process {
            id: self.id.clone(),
            reason: reason.to_string(),
        }
    }
}

impl Processor for ClapProcessor {
    fn layout(&self) -> ChannelLayout {
        ChannelLayout {
            inputs: self.inputs.main_channels(),
            outputs: self.outputs.main_channels(),
        }
    }

    fn latency_frames(&self) -> u32 {
        self.latency
    }

    fn tail(&mut self) -> Tail {
        let (Some(tail), Some(processor)) = (self.extensions.tail, self.processor.as_mut()) else {
            return Tail::Unknown;
        };
        match tail.get(&processor.plugin_handle()) {
            TailLength::Finite(frames) => Tail::Frames(u64::from(frames)),
            TailLength::Infinite => Tail::Infinite,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[Event],
    ) -> Result<(), PluginHostError> {
        let frames = outputs
            .first()
            .map(|c| c.len())
            .or_else(|| inputs.first().map(|c| c.len()))
            .unwrap_or(0);
        if frames == 0 {
            return Ok(());
        }

        if let Some(main) = self.inputs.main {
            for (channel, source) in self.in_bufs[main].iter_mut().zip(inputs) {
                channel[..frames].copy_from_slice(&source[..frames]);
            }
        }
        self.events.clear();
        self.out_events.clear();
        for event in events {
            match *event {
                Event::NoteOn {
                    frame,
                    note_id,
                    key,
                    velocity,
                } => self.events.push(&NoteOnEvent::new(
                    frame,
                    Pckn::new(0u16, 0u16, key.clamp(0, 127) as u16, note_id.max(0) as u32),
                    f64::from(velocity),
                )),
                Event::NoteOff {
                    frame,
                    note_id,
                    key,
                } => self.events.push(&NoteOffEvent::new(
                    frame,
                    Pckn::new(0u16, 0u16, key.clamp(0, 127) as u16, note_id.max(0) as u32),
                    0.0,
                )),
            }
        }

        let Some(processor) = self.processor.as_mut() else {
            return Err(PluginHostError::Process {
                id: self.id.clone(),
                reason: "processor is gone".into(),
            });
        };
        let started = match processor.ensure_processing_started() {
            Ok(started) => started,
            Err(e) => {
                let reason = e.to_string();
                return Err(self.process_error(reason));
            }
        };
        let input_audio = self.in_ports.with_input_buffers(self.in_bufs.iter_mut().map(|port| {
            AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only(
                    port.iter_mut().map(|c| InputChannel::variable(&mut c[..frames])),
                ),
            }
        }));
        let mut output_audio =
            self.out_ports
                .with_output_buffers(self.out_bufs.iter_mut().map(|port| AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_output_only(
                        port.iter_mut().map(|c| &mut c[..frames]),
                    ),
                }));
        let status = started.process(
            &input_audio,
            &mut output_audio,
            &InputEvents::from_buffer(&self.events),
            &mut OutputEvents::from_buffer(&mut self.out_events),
            Some(self.steady_time),
            None,
        );
        if let Err(e) = status {
            let reason = e.to_string();
            return Err(self.process_error(reason));
        }
        self.steady_time += frames as u64;

        match self.outputs.main {
            Some(main) => {
                for (out, channel) in outputs.iter_mut().zip(&self.out_bufs[main]) {
                    out[..frames].copy_from_slice(&channel[..frames]);
                }
            }
            None => {
                for out in outputs.iter_mut() {
                    out.fill(0.0);
                }
            }
        }
        Ok(())
    }
}

impl Drop for ClapProcessor {
    fn drop(&mut self) {
        let Some(processor) = self.processor.take() else {
            return;
        };
        let stopped = processor.into_stopped();
        let instance = self.instance;
        // If the main thread is gone the process is shutting down; the instance goes with it.
        let _ = main_thread::run(move |main| {
            if let Some(mut plugin) = main.instances.remove(&instance) {
                plugin.deactivate(stopped);
            }
        });
    }
}
