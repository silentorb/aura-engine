//! Streaming-mode rendering: sequential blocks of live voices.
//!
//! Deterministic rendering may render any range of a signal. Streaming rendering pulls
//! consecutive blocks from a [`BlockSource`]; time only moves forward, so a voice's signals may
//! keep history. A [`StreamVoice`] is one
//! runtime graph instance sounding one [`LiveNote`]; a [`VoiceMixer`] sums voices started and
//! released from a control thread through a [`VoiceSender`].
//!
//! The mixer neither allocates nor frees on the audio thread: voices are boxed on the control
//! thread, held in preallocated slots, and handed back to the control thread to be dropped once
//! they finish.

use crate::{AudioSignal, Frame, Renderer, BLOCK_FRAMES};
use aura_sample::SampleRate;
use aura_sequencing::LiveNote;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Produces consecutive blocks of stereo frames. Each call continues where the last ended.
pub trait BlockSource: Send {
    /// Fills `out` with the next stereo frames (`[left, right]`); a mono source writes the same
    /// value to both.
    fn render_block(&mut self, out: &mut [[f32; 2]]);
}

/// Most voices sounding at once; starting another releases the oldest.
pub const MAX_VOICES: usize = 16;
/// A released voice below this level for [`QUIET_SECS`] has finished.
const QUIET_LEVEL: f32 = 1e-5;
const QUIET_SECS: f64 = 0.05;
/// A released voice stops after this long regardless of level.
const MAX_TAIL_SECS: f64 = 10.0;
const COMMAND_CAPACITY: usize = 256;
/// Voice slots, held and releasing: preallocated so the audio thread never grows them.
pub const MAX_SLOTS: usize = MAX_VOICES * 2;

/// One runtime graph instance sounding one live note, with its own voice-local clock
/// (onset at `t = 0`).
pub struct StreamVoice {
    root: Renderer,
    live: Arc<LiveNote>,
    frame: u64,
    rate: f64,
    quiet_frames: u64,
}

impl StreamVoice {
    /// `root` is the voice graph, mono or stereo.
    pub fn new(root: AudioSignal, live: Arc<LiveNote>, sample_rate: SampleRate) -> Self {
        Self {
            root: Renderer::new(root),
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

    /// Adds this voice's next `out.len()` frames into `out`, rendered in consecutive blocks.
    pub fn render_add(&mut self, out: &mut [[f32; 2]]) {
        for chunk in out.chunks_mut(BLOCK_FRAMES) {
            let held = self.live.is_held();
            let (left, right) = self.root.block(self.frame as Frame, chunk.len());
            let mut quiet = self.quiet_frames;
            for ((frame, &l), &r) in chunk.iter_mut().zip(left).zip(right) {
                frame[0] += l;
                frame[1] += r;
                if held || l.abs().max(r.abs()) >= QUIET_LEVEL {
                    quiet = 0;
                } else {
                    quiet += 1;
                }
            }
            self.quiet_frames = quiet;
            self.frame += chunk.len() as u64;
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
///
/// Every send first drops the finished voices the mixer handed back; [`VoiceSender::reclaim`]
/// does the same on its own for callers that send rarely.
#[derive(Clone)]
pub struct VoiceSender {
    commands: SyncSender<VoiceCommand>,
    finished: Arc<Mutex<Receiver<Box<StreamVoice>>>>,
}

impl VoiceSender {
    /// Drops voices the mixer has finished with (here, off the audio thread). Returns how many.
    pub fn reclaim(&self) -> usize {
        let Ok(finished) = self.finished.try_lock() else {
            return 0;
        };
        finished.try_iter().count()
    }

    pub fn send(&self, command: VoiceCommand) -> Result<(), VoiceSendError> {
        self.reclaim();
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
    /// Finished voices go back to the control thread, which drops them.
    finished: SyncSender<Box<StreamVoice>>,
    /// `(id, voice)`, oldest first; `id` is `None` once released, so a reused id cannot release
    /// it again. Capacity [`MAX_SLOTS`], never exceeded.
    voices: Vec<(Option<u64>, Box<StreamVoice>)>,
}

impl VoiceMixer {
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// Slots allocated for voices; constant for the mixer's life.
    pub fn slot_capacity(&self) -> usize {
        self.voices.capacity()
    }

    /// Hands a voice back to the control thread. Drops it here only if that queue is full,
    /// which needs more than [`MAX_SLOTS`] voices to finish between two control-thread sends.
    fn retire(&self, voice: Box<StreamVoice>) {
        if let Err(TrySendError::Full(voice) | TrySendError::Disconnected(voice)) =
            self.finished.try_send(voice)
        {
            drop(voice);
        }
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
                // Every slot taken (at most MAX_VOICES held, so the rest are releasing): cut
                // the oldest releasing voice short.
                if self.voices.len() >= MAX_SLOTS {
                    if let Some(index) = self.voices.iter().position(|(id, _)| id.is_none()) {
                        let (_, cut) = self.voices.remove(index);
                        self.retire(cut);
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
    fn render_block(&mut self, out: &mut [[f32; 2]]) {
        while let Ok(command) = self.commands.try_recv() {
            self.apply(command);
        }
        out.fill([0.0; 2]);
        for (_, voice) in &mut self.voices {
            voice.render_add(out);
        }
        let mut index = 0;
        while index < self.voices.len() {
            if self.voices[index].1.is_finished() {
                let (_, done) = self.voices.remove(index);
                self.retire(done);
            } else {
                index += 1;
            }
        }
    }
}

/// A connected control handle and mixer.
pub fn voice_channel() -> (VoiceSender, VoiceMixer) {
    let (commands, receiver) = sync_channel(COMMAND_CAPACITY);
    let (finished, finished_receiver) = sync_channel(MAX_SLOTS);
    (
        VoiceSender {
            commands,
            finished: Arc::new(Mutex::new(finished_receiver)),
        },
        VoiceMixer {
            commands: receiver,
            finished,
            voices: Vec::with_capacity(MAX_SLOTS),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Constant, Signal, StereoPair};
    use aura_sequencing::{NoteParams, Semitone};

    const RATE: SampleRate = SampleRate::RATE_44100;

    /// Full level while held; silent once released.
    struct Gate(Arc<LiveNote>);

    impl Signal for Gate {
        fn render(&mut self, start: Frame, out: &mut [f32]) {
            for (k, sample) in out.iter_mut().enumerate() {
                let t = crate::frame_secs(start + k as Frame, 44_100.0);
                *sample = match self.0.released_after() {
                    Some(released) if t >= released => 0.0,
                    _ => 0.5,
                };
            }
        }
    }

    fn voice() -> StreamVoice {
        let live = Arc::new(LiveNote::new(Semitone(60), 1.0, NoteParams::new()));
        StreamVoice::new(AudioSignal::Mono(Box::new(Gate(Arc::clone(&live)))), live, RATE)
    }

    /// The left channel of the next block (mono voices only, so both channels match).
    fn block(mixer: &mut VoiceMixer, frames: usize) -> Vec<f32> {
        let mut out = vec![[1.0; 2]; frames];
        mixer.render_block(&mut out);
        assert!(out.iter().all(|[left, right]| left == right));
        out.iter().map(|frame| frame[0]).collect()
    }

    #[test]
    fn stereo_voices_keep_their_channels() {
        let (sender, mut mixer) = voice_channel();
        let live = Arc::new(LiveNote::new(Semitone(60), 1.0, NoteParams::new()));
        let stereo = StereoPair::new(Box::new(Constant(0.25)), Box::new(Constant(-0.5)));
        sender
            .note_on(
                1,
                StreamVoice::new(AudioSignal::Stereo(Box::new(stereo)), live, RATE),
            )
            .unwrap();
        sender.note_on(2, voice()).unwrap();
        let mut out = vec![[0.0; 2]; 8];
        mixer.render_block(&mut out);
        assert!(out.iter().all(|frame| *frame == [0.75, 0.0]));
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

    /// Silent from note-on: finishes as soon as it's released.
    fn silent_voice() -> StreamVoice {
        let live = Arc::new(LiveNote::new(Semitone(60), 1.0, NoteParams::new()));
        StreamVoice::new(AudioSignal::Mono(Box::new(Constant(0.0))), live, RATE)
    }

    #[test]
    fn slots_never_grow_under_a_note_on_storm() {
        let (sender, mut mixer) = voice_channel();
        let capacity = mixer.slot_capacity();
        assert_eq!(capacity, MAX_SLOTS);
        for round in 0..8_u64 {
            for id in 0..64_u64 {
                sender.note_on(round * 64 + id, voice()).unwrap_or(());
                if id % 16 == 0 {
                    block(&mut mixer, 8);
                }
            }
            block(&mut mixer, 8);
            assert!(mixer.voice_count() <= MAX_SLOTS);
            assert_eq!(mixer.slot_capacity(), capacity, "no reallocation");
        }
    }

    #[test]
    fn finished_voices_are_handed_back_to_the_control_thread() {
        let (sender, mut mixer) = voice_channel();
        sender.note_on(1, silent_voice()).unwrap();
        sender.note_on(2, silent_voice()).unwrap();
        sender.all_off().unwrap();
        block(&mut mixer, 44_100 / 10);
        assert_eq!(mixer.voice_count(), 0);
        assert_eq!(sender.reclaim(), 2, "dropped on the control thread");
        assert_eq!(sender.reclaim(), 0);
    }
}
