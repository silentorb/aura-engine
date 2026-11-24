//! Streaming-mode rendering: sequential blocks of live voices.
//!
//! Deterministic rendering samples a [`Sampler`] at any time. Streaming rendering pulls
//! consecutive blocks from a [`BlockSource`]; time only moves forward. A [`StreamVoice`] is one
//! runtime graph instance sounding one [`LiveNote`]; a [`VoiceMixer`] sums voices started and
//! released from a control thread through a [`VoiceSender`].

use crate::Sampler;
use aura_sample::SampleRate;
use aura_sequencing::LiveNote;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use thiserror::Error;

/// Produces consecutive blocks of mono samples. Each call continues where the last ended.
pub trait BlockSource: Send {
    fn render_block(&mut self, out: &mut [f32]);
}

/// Most voices sounding at once; starting another releases the oldest.
pub const MAX_VOICES: usize = 16;
/// A released voice below this level for [`QUIET_SECS`] has finished.
const QUIET_LEVEL: f32 = 1e-5;
const QUIET_SECS: f64 = 0.05;
/// A released voice stops after this long regardless of level.
const MAX_TAIL_SECS: f64 = 10.0;
const COMMAND_CAPACITY: usize = 256;

/// One runtime graph instance sounding one live note, with its own voice-local clock
/// (onset at `t = 0`).
pub struct StreamVoice {
    root: Box<dyn Sampler>,
    live: Arc<LiveNote>,
    frame: u64,
    rate: f64,
    quiet_frames: u64,
}

impl StreamVoice {
    pub fn new(root: Box<dyn Sampler>, live: Arc<LiveNote>, sample_rate: SampleRate) -> Self {
        Self {
            root,
            live,
            frame: 0,
            rate: f64::from(sample_rate.get()),
            quiet_frames: 0,
        }
    }

    fn now(&self) -> f64 {
        self.frame as f64 / self.rate
    }

    /// Note-off: stamps the release time on the live note.
    pub fn release(&mut self) {
        self.live.release(self.now());
    }

    pub fn is_finished(&self) -> bool {
        let Some(released) = self.live.released_after() else {
            return false;
        };
        self.now() - released >= MAX_TAIL_SECS || self.quiet_frames as f64 / self.rate >= QUIET_SECS
    }

    /// Adds this voice's next `out.len()` samples into `out`.
    pub fn render_add(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            let value = self.root.at(self.now());
            *sample += value;
            self.frame += 1;
            if self.live.is_held() || value.abs() >= QUIET_LEVEL {
                self.quiet_frames = 0;
            } else {
                self.quiet_frames += 1;
            }
        }
    }
}

/// Commands from the control thread to a [`VoiceMixer`].
pub enum VoiceCommand {
    /// Start a voice under `id` (releasing any voice already under `id`).
    NoteOn {
        id: u64,
        voice: Box<StreamVoice>,
    },
    NoteOff {
        id: u64,
    },
    AllOff,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VoiceSendError {
    #[error("voice command queue is full")]
    Full,
    #[error("voice mixer is gone")]
    Disconnected,
}

/// Control-thread handle for a [`VoiceMixer`]. Never blocks.
#[derive(Clone)]
pub struct VoiceSender {
    commands: SyncSender<VoiceCommand>,
}

impl VoiceSender {
    pub fn send(&self, command: VoiceCommand) -> Result<(), VoiceSendError> {
        self.commands.try_send(command).map_err(|err| match err {
            TrySendError::Full(_) => VoiceSendError::Full,
            TrySendError::Disconnected(_) => VoiceSendError::Disconnected,
        })
    }

    pub fn note_on(&self, id: u64, voice: StreamVoice) -> Result<(), VoiceSendError> {
        self.send(VoiceCommand::NoteOn {
            id,
            voice: Box::new(voice),
        })
    }

    pub fn note_off(&self, id: u64) -> Result<(), VoiceSendError> {
        self.send(VoiceCommand::NoteOff { id })
    }

    pub fn all_off(&self) -> Result<(), VoiceSendError> {
        self.send(VoiceCommand::AllOff)
    }
}

/// Sums live voices; runs on the audio thread. Commands are drained without blocking at the
/// start of each block.
pub struct VoiceMixer {
    commands: Receiver<VoiceCommand>,
    /// `(id, voice)`; `id` is `None` once released, so a reused id cannot release it again.
    voices: Vec<(Option<u64>, Box<StreamVoice>)>,
}

impl VoiceMixer {
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    fn apply(&mut self, command: VoiceCommand) {
        match command {
            VoiceCommand::NoteOn { id, voice } => {
                self.release_id(id);
                let held = self.voices.iter().filter(|(id, _)| id.is_some()).count();
                if held >= MAX_VOICES {
                    if let Some((oldest, voice)) =
                        self.voices.iter_mut().find(|(id, _)| id.is_some())
                    {
                        *oldest = None;
                        voice.release();
                    }
                }
                self.voices.push((Some(id), voice));
            }
            VoiceCommand::NoteOff { id } => self.release_id(id),
            VoiceCommand::AllOff => {
                for (id, voice) in &mut self.voices {
                    if id.take().is_some() {
                        voice.release();
                    }
                }
            }
        }
    }

    fn release_id(&mut self, target: u64) {
        for (id, voice) in &mut self.voices {
            if *id == Some(target) {
                *id = None;
                voice.release();
            }
        }
    }
}

impl BlockSource for VoiceMixer {
    fn render_block(&mut self, out: &mut [f32]) {
        while let Ok(command) = self.commands.try_recv() {
            self.apply(command);
        }
        out.fill(0.0);
        for (_, voice) in &mut self.voices {
            voice.render_add(out);
        }
        self.voices.retain(|(_, voice)| !voice.is_finished());
    }
}

/// A connected control handle and mixer.
pub fn voice_channel() -> (VoiceSender, VoiceMixer) {
    let (commands, receiver) = sync_channel(COMMAND_CAPACITY);
    (
        VoiceSender { commands },
        VoiceMixer {
            commands: receiver,
            voices: Vec::with_capacity(MAX_VOICES * 2),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_sequencing::{NoteParams, Semitone};

    const RATE: SampleRate = SampleRate::RATE_44100;

    /// Full level while held; silent once released.
    struct Gate(Arc<LiveNote>);

    impl Sampler for Gate {
        fn at(&self, t: f64) -> f32 {
            match self.0.released_after() {
                Some(released) if t >= released => 0.0,
                _ => 0.5,
            }
        }
    }

    fn voice() -> StreamVoice {
        let live = Arc::new(LiveNote::new(Semitone(60), 1.0, NoteParams::new()));
        StreamVoice::new(Box::new(Gate(Arc::clone(&live))), live, RATE)
    }

    fn block(mixer: &mut VoiceMixer, frames: usize) -> Vec<f32> {
        let mut out = vec![1.0; frames];
        mixer.render_block(&mut out);
        out
    }

    #[test]
    fn voices_mix_and_retire_after_release() {
        let (sender, mut mixer) = voice_channel();
        assert!(block(&mut mixer, 64).iter().all(|&s| s == 0.0));
        sender.note_on(1, voice()).unwrap();
        sender.note_on(2, voice()).unwrap();
        assert!(block(&mut mixer, 64)
            .iter()
            .all(|&s| (s - 1.0).abs() < 1e-6));
        sender.note_off(1).unwrap();
        let after = block(&mut mixer, 64);
        assert!(after.iter().all(|&s| (s - 0.5).abs() < 1e-6));
        block(&mut mixer, 44_100 / 10);
        assert_eq!(mixer.voice_count(), 1, "released voice retired once quiet");
    }

    #[test]
    fn reusing_an_id_releases_the_previous_voice() {
        let (sender, mut mixer) = voice_channel();
        sender.note_on(7, voice()).unwrap();
        sender.note_on(7, voice()).unwrap();
        block(&mut mixer, 44_100 / 10);
        assert_eq!(mixer.voice_count(), 1);
    }

    #[test]
    fn polyphony_is_capped_by_releasing_the_oldest() {
        let (sender, mut mixer) = voice_channel();
        for id in 0..(MAX_VOICES as u64 + 4) {
            sender.note_on(id, voice()).unwrap();
        }
        block(&mut mixer, 44_100 / 10);
        assert_eq!(mixer.voice_count(), MAX_VOICES);
    }

    #[test]
    fn all_off_releases_everything() {
        let (sender, mut mixer) = voice_channel();
        sender.note_on(1, voice()).unwrap();
        sender.note_on(2, voice()).unwrap();
        block(&mut mixer, 16);
        sender.all_off().unwrap();
        block(&mut mixer, 44_100 / 10);
        assert_eq!(mixer.voice_count(), 0);
    }
}
