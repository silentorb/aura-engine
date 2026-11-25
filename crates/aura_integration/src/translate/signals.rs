//! Block signals of Aura's deterministic node types.
//!
//! Each computes, frame by frame, exactly the value the node's function gives at that frame's
//! time (`aura_render::frame_secs`), so a range renders the same whatever was rendered before.

use super::note_schedule::NoteSchedule;
use aura_dsp::{
    add, bandlimited_saw, deterministic_noise, exponential_sweep_sine, highpass_noise, multiply,
    pan_gains, sine,
};
use aura_instrumentation::LinearAdsr;
use aura_render::{frame_secs, Frame, Input, Signal, StereoSignal};
use aura_sequencing::{ChordSignal, ProgressionSignal, Semitone, Tempo};

/// The time of each frame in `start .. start + len`, in seconds.
fn times(start: Frame, len: usize, rate: f64) -> impl Iterator<Item = f64> {
    (0..len).map(move |k| frame_secs(start + k as Frame, rate))
}

pub(super) struct TimeSignal {
    pub(super) rate: f64,
}

impl Signal for TimeSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        for (k, sample) in out.iter_mut().enumerate() {
            let t = frame_secs(start + k as Frame, self.rate);
            *sample = t as f32;
        }
    }
}

pub(super) struct TimeElapsedSignal {
    pub(super) start: Input,
    pub(super) rate: f64,
}

impl Signal for TimeElapsedSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let note_start = self.start.read(start, len);
        for ((sample, t), &from) in out.iter_mut().zip(times(start, len, self.rate)).zip(note_start) {
            *sample = (t - from as f64).max(0.0) as f32;
        }
    }
}

pub(super) struct SineSignal {
    pub(super) frequency: Input,
    pub(super) time: Input,
}

impl Signal for SineSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let frequency = self.frequency.read(start, len);
        let time = self.time.read(start, len);
        for ((sample, &f), &t) in out.iter_mut().zip(frequency).zip(time) {
            *sample = sine(f, t as f64);
        }
    }
}

pub(super) struct MultiplySignal {
    pub(super) a: Input,
    pub(super) b: Input,
}

impl Signal for MultiplySignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let a = self.a.read(start, len);
        let b = self.b.read(start, len);
        for ((sample, &a), &b) in out.iter_mut().zip(a).zip(b) {
            *sample = multiply(a, b);
        }
    }
}

/// Sums its inputs in edge order.
pub(super) struct AddSignal {
    pub(super) inputs: Vec<Input>,
}

impl Signal for AddSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let mut inputs = self.inputs.iter_mut();
        match inputs.next() {
            Some(first) => out.copy_from_slice(first.read(start, len)),
            None => out.fill(0.0),
        }
        for input in inputs {
            for (sample, &value) in out.iter_mut().zip(input.read(start, len)) {
                *sample = add(*sample, value);
            }
        }
    }
}

pub(super) struct ExponentialSweepSineSignal {
    pub(super) start_hz: Input,
    pub(super) decay_rate: Input,
    pub(super) elapsed: Input,
}

impl Signal for ExponentialSweepSineSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let start_hz = self.start_hz.read(start, len);
        let decay_rate = self.decay_rate.read(start, len);
        let elapsed = self.elapsed.read(start, len);
        for (k, sample) in out.iter_mut().enumerate() {
            *sample = exponential_sweep_sine(start_hz[k], decay_rate[k], elapsed[k] as f64);
        }
    }
}

pub(super) struct DeterministicNoiseSignal {
    pub(super) seed: Input,
    pub(super) time: Input,
}

impl Signal for DeterministicNoiseSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let seed = self.seed.read(start, len);
        let time = self.time.read(start, len);
        for ((sample, &seed), &t) in out.iter_mut().zip(seed).zip(time) {
            *sample = deterministic_noise(seed, t as f64);
        }
    }
}

pub(super) struct HighpassNoiseSignal {
    pub(super) seed: Input,
    pub(super) time: Input,
    pub(super) dt: Input,
}

impl Signal for HighpassNoiseSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let seed = self.seed.read(start, len);
        let time = self.time.read(start, len);
        let dt = self.dt.read(start, len);
        for (k, sample) in out.iter_mut().enumerate() {
            *sample = highpass_noise(seed[k], time[k] as f64, dt[k] as f64);
        }
    }
}

pub(super) struct LinearAdsrSignal {
    pub(super) attack: Input,
    pub(super) decay: Input,
    pub(super) sustain: Input,
    pub(super) release: Input,
    pub(super) elapsed: Input,
    pub(super) note_duration: Input,
}

impl Signal for LinearAdsrSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let attack = self.attack.read(start, len);
        let decay = self.decay.read(start, len);
        let sustain = self.sustain.read(start, len);
        let release = self.release.read(start, len);
        let elapsed = self.elapsed.read(start, len);
        let note_duration = self.note_duration.read(start, len);
        for (k, sample) in out.iter_mut().enumerate() {
            let adsr = LinearAdsr {
                attack: attack[k] as f64,
                decay: decay[k] as f64,
                sustain: sustain[k],
                release: release[k] as f64,
            };
            *sample = adsr.value_at(elapsed[k] as f64, note_duration[k] as f64);
        }
    }
}

pub(super) struct SemitoneToHzSignal {
    pub(super) semitone: Input,
}

impl Signal for SemitoneToHzSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let semitone = self.semitone.read(start, out.len());
        for (sample, &value) in out.iter_mut().zip(semitone) {
            *sample = Semitone(value.round() as i16).to_hz() as f32;
        }
    }
}

/// Which fact about the active note a [`NoteFieldSignal`] reads.
#[derive(Debug, Clone)]
pub(super) enum NoteField {
    Semitone,
    Active,
    Start,
    Duration,
    Velocity,
    Param(String),
}

/// A fact about the score's active note at each frame.
pub(super) struct NoteFieldSignal {
    pub(super) schedule: NoteSchedule,
    pub(super) field: NoteField,
    pub(super) rate: f64,
}

impl Signal for NoteFieldSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let schedule = &self.schedule;
        for (k, sample) in out.iter_mut().enumerate() {
            let t = frame_secs(start + k as Frame, self.rate);
            *sample = match &self.field {
                NoteField::Semitone => schedule
                    .active_at(t)
                    .map(|note| note.semitone.0 as f32)
                    .unwrap_or(0.0),
                NoteField::Active => {
                    if schedule.active_at(t).is_some() {
                        1.0
                    } else {
                        0.0
                    }
                }
                NoteField::Start => schedule
                    .note_global_start_secs(t)
                    .map(|start| start as f32)
                    .unwrap_or(0.0),
                NoteField::Duration => schedule
                    .active_at(t)
                    .map(|note| schedule.note_duration_secs(note) as f32)
                    .unwrap_or(0.0),
                NoteField::Velocity => schedule
                    .active_at(t)
                    .map(|note| note.velocity)
                    .unwrap_or(0.0),
                NoteField::Param(name) => match schedule.active_at(t) {
                    None => 0.0,
                    Some(note) => note.param_or(name, 1.0) as f32,
                },
            };
        }
    }
}

pub(super) struct BandlimitedSawSignal {
    pub(super) frequency: Input,
    pub(super) elapsed: Input,
    pub(super) cutoff: Input,
}

impl Signal for BandlimitedSawSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let len = out.len();
        let frequency = self.frequency.read(start, len);
        let elapsed = self.elapsed.read(start, len);
        let cutoff = self.cutoff.read(start, len);
        for (k, sample) in out.iter_mut().enumerate() {
            *sample = bandlimited_saw(frequency[k], elapsed[k] as f64, cutoff[k]);
        }
    }
}

pub(super) struct NoteEnvelopeSignal {
    pub(super) schedule: NoteSchedule,
    pub(super) adsr: LinearAdsr,
    pub(super) rate: f64,
}

impl Signal for NoteEnvelopeSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let schedule = &self.schedule;
        for (k, sample) in out.iter_mut().enumerate() {
            let t = frame_secs(start + k as Frame, self.rate);
            *sample = match schedule.note_local_secs(t) {
                None => 0.0,
                Some(local) => {
                    let duration = schedule
                        .active_at(t)
                        .map(|note| schedule.note_duration_secs(note))
                        .unwrap_or(0.0);
                    self.adsr.value_at(local, duration)
                }
            };
        }
    }
}

/// Polyphonic sine voice: mixes overlapping notes (sine × envelope × velocity).
pub(super) struct PolySineScoreSignal {
    pub(super) schedule: NoteSchedule,
    pub(super) adsr: LinearAdsr,
    pub(super) rate: f64,
}

impl Signal for PolySineScoreSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let schedule = &self.schedule;
        for (k, sample) in out.iter_mut().enumerate() {
            let t = frame_secs(start + k as Frame, self.rate);
            let mut mix = 0.0f32;
            for note in schedule.overlapping_at(t) {
                let local = schedule.note_local_secs_for(note, t);
                let duration = schedule.note_duration_secs(note);
                let env = self.adsr.value_at(local, duration);
                let frequency = note.semitone.to_hz() as f32;
                mix += sine(frequency, local) * env * note.velocity;
            }
            *sample = mix;
        }
    }
}

pub(super) struct ChordRootSignal {
    pub(super) progression: ProgressionSignal,
    pub(super) tempo: Tempo,
    pub(super) rate: f64,
}

impl Signal for ChordRootSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        for (k, sample) in out.iter_mut().enumerate() {
            let t = frame_secs(start + k as Frame, self.rate);
            *sample = self.progression.chord_at(t, self.tempo).root.0 as f32;
        }
    }
}

pub(super) struct ChordActiveSignal {
    pub(super) progression: ProgressionSignal,
    pub(super) tempo: Tempo,
    pub(super) rate: f64,
}

impl Signal for ChordActiveSignal {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        if self.progression.looping {
            out.fill(1.0);
            return;
        }
        for (k, sample) in out.iter_mut().enumerate() {
            let t = frame_secs(start + k as Frame, self.rate);
            let beat = t / self.tempo.seconds_per_beat();
            let active = self.progression.progression.regions.iter().any(|region| {
                beat >= region.start_beats - 1e-9
                    && beat < region.start_beats + region.duration_beats - 1e-9
            });
            *sample = if active { 1.0 } else { 0.0 };
        }
    }
}

/// Equal-power pan of a mono signal.
pub(super) struct PanSignal {
    pub(super) audio: Input,
    pub(super) position: Input,
}

impl StereoSignal for PanSignal {
    fn render(&mut self, start: Frame, left: &mut [f32], right: &mut [f32]) {
        let len = left.len();
        let audio = self.audio.read(start, len);
        let position = self.position.read(start, len);
        for k in 0..len {
            let (gain_left, gain_right) = pan_gains(position[k]);
            left[k] = audio[k] * gain_left;
            right[k] = audio[k] * gain_right;
        }
    }
}
