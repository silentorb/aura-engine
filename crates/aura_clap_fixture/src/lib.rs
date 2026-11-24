//! Deterministic CLAP plugins for testing Aura's plugin host.
//!
//! - [`SINE_ID`] — a polyphonic sine synth: note-id aware, velocity-scaled, with a `Level`
//!   parameter ([`SINE_LEVEL_PARAM`], default [`DEFAULT_SINE_LEVEL`]), a linear
//!   release of [`SINE_RELEASE_SECS`] and a fixed output latency of [`SINE_LATENCY`] frames.
//!   Stereo output, no audio input.
//! - [`GAIN_ID`] — a stereo gain. Its gain lives in the plugin state (4 bytes, `f32`
//!   little-endian), default [`DEFAULT_GAIN`].
//!
//! Built as a `cdylib`, this is an ordinary `.clap` bundle (rename the library to
//! `aura_clap_fixture.clap`). Tests load [`FixtureEntry`] in-process instead.

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::latency::{PluginLatency, PluginLatencyImpl};
use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginAudioProcessorParams,
    PluginMainThreadParams, PluginParams,
};
use clack_extensions::note_ports::{
    NoteDialect, NoteDialects, NotePortInfo, NotePortInfoWriter, PluginNotePorts,
    PluginNotePortsImpl,
};
use clack_extensions::state::{PluginState, PluginStateImpl};
use clack_extensions::tail::{PluginTail, PluginTailImpl, TailLength};
use clack_plugin::entry::prelude::*;
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use clack_plugin::events::io::{InputEvents, OutputEvents};
use clack_plugin::stream::{InputStream, OutputStream};
use std::ffi::CStr;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};

pub const SINE_ID: &str = "aura.fixture.sine";
pub const GAIN_ID: &str = "aura.fixture.gain";
pub const SINE_LATENCY: u32 = 64;
pub const SINE_RELEASE_SECS: f64 = 0.1;
/// Parameter id of the sine's output level (0–1).
pub const SINE_LEVEL_PARAM: u32 = 0;
pub const DEFAULT_SINE_LEVEL: f32 = 0.25;
pub const DEFAULT_GAIN: f32 = 0.5;
const MAX_FRAMES: usize = 4096;

/// Encodes a gain as [`GAIN_ID`]'s state blob.
pub fn gain_state(gain: f32) -> Vec<u8> {
    gain.to_le_bytes().to_vec()
}

fn stereo_port(writer: &mut AudioPortInfoWriter, name: &[u8]) {
    writer.set(&AudioPortInfo {
        id: ClapId::new(0),
        name,
        channel_count: 2,
        flags: AudioPortFlags::IS_MAIN,
        port_type: Some(AudioPortType::STEREO),
        in_place_pair: None,
    });
}

// ---------------------------------------------------------------------------------------------
// Sine synth

pub struct FixtureSine;

/// The sine's `level` parameter (output gain), shared between threads.
pub struct SineShared {
    level_bits: AtomicU32,
}

impl PluginShared<'_> for SineShared {}

impl SineShared {
    fn level(&self) -> f32 {
        f32::from_bits(self.level_bits.load(Ordering::Relaxed))
    }

    fn set_level(&self, level: f64) {
        let level = level.clamp(0.0, 1.0) as f32;
        self.level_bits.store(level.to_bits(), Ordering::Relaxed);
    }
}

pub struct SineMainThread<'a> {
    shared: &'a SineShared,
}

impl<'a> PluginMainThread<'a, SineShared> for SineMainThread<'a> {}

impl PluginMainThreadParams for SineMainThread<'_> {
    fn count(&self) -> u32 {
        1
    }

    fn get_info(&self, param_index: u32, info: &mut ParamInfoWriter) {
        if param_index == 0 {
            info.set(&ParamInfo {
                id: ClapId::new(SINE_LEVEL_PARAM),
                flags: ParamInfoFlags::IS_AUTOMATABLE,
                cookie: Default::default(),
                name: b"Level",
                module: b"Output",
                min_value: 0.0,
                max_value: 1.0,
                default_value: f64::from(DEFAULT_SINE_LEVEL),
            });
        }
    }

    fn get_value(&self, param_id: ClapId) -> Option<f64> {
        (param_id.get() == SINE_LEVEL_PARAM).then(|| f64::from(self.shared.level()))
    }

    fn value_to_text(
        &self,
        _param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        use std::fmt::Write as _;
        write!(writer, "{value:.3}")
    }

    fn text_to_value(&self, _param_id: ClapId, text: &CStr) -> Option<f64> {
        text.to_str().ok()?.trim().parse().ok()
    }

    fn flush(&self, input: &InputEvents, _output: &mut OutputEvents) {
        apply_level_events(self.shared, input);
    }
}

fn apply_level_events(shared: &SineShared, input: &InputEvents) {
    for event in input {
        if let Some(CoreEventSpace::ParamValue(change)) = event.as_core_event() {
            if change.param_id().map(|id| id.get()) == Some(SINE_LEVEL_PARAM) {
                shared.set_level(change.value());
            }
        }
    }
}

impl PluginAudioPortsImpl for SineMainThread<'_> {
    fn count(&self, is_input: bool) -> u32 {
        if is_input {
            0
        } else {
            1
        }
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if !is_input && index == 0 {
            stereo_port(writer, b"main");
        }
    }
}

impl PluginNotePortsImpl for SineMainThread<'_> {
    fn count(&self, is_input: bool) -> u32 {
        if is_input {
            1
        } else {
            0
        }
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut NotePortInfoWriter) {
        if is_input && index == 0 {
            writer.set(&NotePortInfo {
                id: ClapId::new(0),
                name: b"notes",
                supported_dialects: NoteDialects::CLAP,
                preferred_dialect: Some(NoteDialect::Clap),
            });
        }
    }
}

impl PluginLatencyImpl for SineMainThread<'_> {
    fn get(&self) -> u32 {
        SINE_LATENCY
    }
}

impl Plugin for FixtureSine {
    type AudioProcessor<'a> = SineProcessor<'a>;
    type Shared<'a> = SineShared;
    type MainThread<'a> = SineMainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&SineShared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginParams>()
            .register::<PluginNotePorts>()
            .register::<PluginLatency>()
            .register::<PluginTail>();
    }
}

impl DefaultPluginFactory for FixtureSine {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::*;
        PluginDescriptor::new(SINE_ID, "Aura Fixture Sine")
            .with_vendor("Aura")
            .with_version("1.0.0")
            .with_features([INSTRUMENT, SYNTHESIZER, STEREO])
    }

    fn new_shared(_host: HostSharedHandle<'_>) -> Result<SineShared, PluginError> {
        Ok(SineShared {
            level_bits: AtomicU32::new(DEFAULT_SINE_LEVEL.to_bits()),
        })
    }

    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        shared: &'a SineShared,
    ) -> Result<SineMainThread<'a>, PluginError> {
        Ok(SineMainThread { shared })
    }
}

#[derive(Clone, Copy)]
struct Voice {
    note_id: i32,
    key: i16,
    velocity: f32,
    /// Frames since note-on.
    age: u64,
    /// Frames since note-off, once released.
    released: Option<u64>,
}

pub struct SineProcessor<'a> {
    shared: &'a SineShared,
    rate: f64,
    voices: Vec<Voice>,
    /// Output delay line of [`SINE_LATENCY`] frames.
    delay: Vec<f32>,
    delay_pos: usize,
}

impl SineProcessor<'_> {
    fn note_on(&mut self, note_id: i32, key: i16, velocity: f32) {
        if self.voices.len() < self.voices.capacity() {
            self.voices.push(Voice {
                note_id,
                key,
                velocity,
                age: 0,
                released: None,
            });
        }
    }

    fn note_off(&mut self, note_id: i32, key: i16) {
        for voice in &mut self.voices {
            let matches = if note_id >= 0 {
                voice.note_id == note_id
            } else {
                voice.key == key
            };
            if matches && voice.released.is_none() {
                voice.released = Some(0);
            }
        }
    }

    fn next_sample(&mut self) -> f32 {
        let release_frames = SINE_RELEASE_SECS * self.rate;
        let mut sum = 0.0_f64;
        for voice in &mut self.voices {
            let hz = 440.0 * 2.0_f64.powf(f64::from(voice.key - 69) / 12.0);
            let t = voice.age as f64 / self.rate;
            let env = match voice.released {
                None => 1.0,
                Some(r) => (1.0 - r as f64 / release_frames).max(0.0),
            };
            sum += (2.0 * std::f64::consts::PI * hz * t).sin() * env * f64::from(voice.velocity);
            voice.age += 1;
            if let Some(r) = &mut voice.released {
                *r += 1;
            }
        }
        self.voices
            .retain(|v| v.released.is_none_or(|r| (r as f64) < release_frames));
        let dry = sum as f32 * self.shared.level();
        let wet = self.delay[self.delay_pos];
        self.delay[self.delay_pos] = dry;
        self.delay_pos = (self.delay_pos + 1) % self.delay.len();
        wet
    }
}

impl PluginTailImpl for SineProcessor<'_> {
    fn get(&self) -> TailLength {
        TailLength::Finite((SINE_RELEASE_SECS * self.rate) as u32)
    }
}

impl PluginAudioProcessorParams for SineProcessor<'_> {
    fn flush(&mut self, input: &InputEvents, _output: &mut OutputEvents) {
        apply_level_events(self.shared, input);
    }
}

impl<'a> PluginAudioProcessor<'a, SineShared, SineMainThread<'a>> for SineProcessor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &SineMainThread<'a>,
        shared: &'a SineShared,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Ok(Self {
            shared,
            rate: audio_config.sample_rate,
            voices: Vec::with_capacity(64),
            delay: vec![0.0; SINE_LATENCY as usize],
            delay_pos: 0,
        })
    }

    fn process(
        &mut self,
        _process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        let frames = audio.frames_count() as usize;
        if frames > MAX_FRAMES {
            return Err(PluginError::Message("block larger than the fixture supports"));
        }
        let mut mono = [0.0_f32; MAX_FRAMES];
        let mut pending = events.input.iter().peekable();
        for (frame, out) in mono.iter_mut().enumerate().take(frames) {
            while let Some(event) = pending.next_if(|e| e.header().time() as usize <= frame) {
                match event.as_core_event() {
                    Some(CoreEventSpace::NoteOn(on)) => {
                        self.note_on(on.note_id().to_raw(), on.key().to_raw(), on.velocity() as f32);
                    }
                    Some(CoreEventSpace::NoteOff(off)) => {
                        self.note_off(off.note_id().to_raw(), off.key().to_raw());
                    }
                    Some(CoreEventSpace::ParamValue(change))
                        if change.param_id().map(|id| id.get()) == Some(SINE_LEVEL_PARAM) =>
                    {
                        self.shared.set_level(change.value());
                    }
                    _ => {}
                }
            }
            *out = self.next_sample();
        }
        let mut port = audio
            .output_port(0)
            .ok_or(PluginError::Message("no output port"))?;
        let mut channels = port
            .channels()?
            .into_f32()
            .ok_or(PluginError::Message("expected f32 output"))?;
        for channel in channels.iter_mut() {
            channel[..frames].copy_from_slice(&mono[..frames]);
        }
        Ok(ProcessStatus::Continue)
    }

    fn reset(&mut self) {
        self.voices.clear();
        self.delay.fill(0.0);
        self.delay_pos = 0;
    }
}

// ---------------------------------------------------------------------------------------------
// Gain

pub struct FixtureGain;

pub struct GainShared {
    gain_bits: AtomicU32,
}

impl PluginShared<'_> for GainShared {}

impl GainShared {
    fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }
}

pub struct GainMainThread<'a> {
    shared: &'a GainShared,
}

impl<'a> PluginMainThread<'a, GainShared> for GainMainThread<'a> {}

impl PluginAudioPortsImpl for GainMainThread<'_> {
    fn count(&self, _is_input: bool) -> u32 {
        1
    }

    fn get(&self, index: u32, _is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index == 0 {
            stereo_port(writer, b"main");
        }
    }
}

impl PluginStateImpl for GainMainThread<'_> {
    fn save(&self, output: &mut OutputStream) -> Result<(), PluginError> {
        output.write_all(&gain_state(self.shared.gain()))?;
        Ok(())
    }

    fn load(&self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut bytes = [0u8; 4];
        input.read_exact(&mut bytes)?;
        let gain = f32::from_le_bytes(bytes);
        if !gain.is_finite() {
            return Err(PluginError::Message("gain must be finite"));
        }
        self.shared.gain_bits.store(gain.to_bits(), Ordering::Relaxed);
        Ok(())
    }
}

impl Plugin for FixtureGain {
    type AudioProcessor<'a> = GainProcessor<'a>;
    type Shared<'a> = GainShared;
    type MainThread<'a> = GainMainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&GainShared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginState>();
    }
}

impl DefaultPluginFactory for FixtureGain {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::*;
        PluginDescriptor::new(GAIN_ID, "Aura Fixture Gain")
            .with_vendor("Aura")
            .with_version("1.0.0")
            .with_features([AUDIO_EFFECT, STEREO])
    }

    fn new_shared(_host: HostSharedHandle<'_>) -> Result<GainShared, PluginError> {
        Ok(GainShared {
            gain_bits: AtomicU32::new(DEFAULT_GAIN.to_bits()),
        })
    }

    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        shared: &'a GainShared,
    ) -> Result<GainMainThread<'a>, PluginError> {
        Ok(GainMainThread { shared })
    }
}

pub struct GainProcessor<'a> {
    shared: &'a GainShared,
}

impl<'a> PluginAudioProcessor<'a, GainShared, GainMainThread<'a>> for GainProcessor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &GainMainThread<'a>,
        shared: &'a GainShared,
        _audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Ok(Self { shared })
    }

    fn process(
        &mut self,
        _process: Process,
        mut audio: Audio,
        _events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        let gain = self.shared.gain();
        for mut pair in &mut audio {
            let Some(channels) = pair.channels()?.into_f32() else {
                continue;
            };
            for channel in channels {
                match channel {
                    ChannelPair::InputOnly(_) => {}
                    ChannelPair::OutputOnly(out) => out.fill(0.0),
                    ChannelPair::InputOutput(input, output) => {
                        for (i, o) in input.iter().zip(output) {
                            *o = i * gain;
                        }
                    }
                    ChannelPair::InPlace(buf) => {
                        for sample in buf {
                            *sample *= gain;
                        }
                    }
                }
            }
        }
        Ok(ProcessStatus::Continue)
    }
}

// ---------------------------------------------------------------------------------------------
// Entry: both plugins in one bundle

pub struct FixtureEntry {
    factory: PluginFactoryWrapper<FixtureFactory>,
}

pub struct FixtureFactory {
    sine: PluginDescriptor,
    gain: PluginDescriptor,
}

impl PluginFactoryImpl for FixtureFactory {
    fn plugin_count(&self) -> u32 {
        2
    }

    fn plugin_descriptor(&self, index: u32) -> Option<&PluginDescriptor> {
        match index {
            0 => Some(&self.sine),
            1 => Some(&self.gain),
            _ => None,
        }
    }

    fn create_plugin<'a>(
        &'a self,
        host_info: HostInfo<'a>,
        plugin_id: &CStr,
    ) -> Option<PluginInstance<'a>> {
        let id = plugin_id.to_bytes();
        if id == SINE_ID.as_bytes() {
            Some(PluginInstance::new::<FixtureSine>(
                host_info,
                &self.sine,
                FixtureSine::new_shared,
                FixtureSine::new_main_thread,
            ))
        } else if id == GAIN_ID.as_bytes() {
            Some(PluginInstance::new::<FixtureGain>(
                host_info,
                &self.gain,
                FixtureGain::new_shared,
                FixtureGain::new_main_thread,
            ))
        } else {
            None
        }
    }
}

impl Entry for FixtureEntry {
    fn new(_bundle_path: Option<&CStr>) -> Result<Self, EntryLoadError> {
        Ok(Self {
            factory: PluginFactoryWrapper::new(FixtureFactory {
                sine: FixtureSine::get_descriptor(),
                gain: FixtureGain::get_descriptor(),
            }),
        })
    }

    fn declare_factories<'a>(&'a self, builder: &mut EntryFactories<'a>) {
        builder.register_factory(&self.factory);
    }
}

clack_export_entry!(FixtureEntry);
