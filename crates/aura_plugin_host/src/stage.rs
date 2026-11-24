//! A shared plugin instance in streaming mode.

use crate::{sort_events, Event, MonoStage, BLOCK_FRAMES};
use aura_render::BlockSource;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};

const COMMAND_CAPACITY: usize = 256;
/// Most notes a stage tracks as held at once; another note-on releases the oldest.
const MAX_HELD: usize = 128;

/// Note commands from a control thread to a [`PluginStage`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StageCommand {
    /// Starts a note under `id` (releasing any note already under `id`).
    NoteOn { id: u64, key: i16, velocity: f32 },
    NoteOff { id: u64 },
    AllOff,
}

/// Control-thread handle for a [`PluginStage`]. Never blocks.
#[derive(Clone)]
pub struct StageSender {
    commands: SyncSender<StageCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StageSendError {
    #[error("plugin stage command queue is full")]
    Full,
    #[error("plugin stage is gone")]
    Disconnected,
}

impl StageSender {
    pub fn send(&self, command: StageCommand) -> Result<(), StageSendError> {
        self.commands.try_send(command).map_err(|err| match err {
            TrySendError::Full(_) => StageSendError::Full,
            TrySendError::Disconnected(_) => StageSendError::Disconnected,
        })
    }
}

/// One plugin instance shared by every note, as a [`BlockSource`].
///
/// An instrument stage receives note commands through its [`StageSender`]; an effect stage
/// pulls its audio input from another `BlockSource` (a voice mixer or another stage). Commands
/// are drained at the start of each block and take effect at its first frame. Nothing is
/// allocated on the audio thread.
pub struct PluginStage {
    plugin_id: String,
    stage: MonoStage,
    commands: Receiver<StageCommand>,
    input: Option<Box<dyn BlockSource>>,
    /// `(id, note_id, key)` of held notes, oldest first.
    held: Vec<(u64, i32, i16)>,
    next_note_id: i32,
    events: Vec<Event>,
    in_buf: Vec<f32>,
    failed: bool,
}

/// A connected control handle and stage. `input` makes it an effect over that source.
pub fn plugin_stage(
    plugin_id: impl Into<String>,
    stage: MonoStage,
    input: Option<Box<dyn BlockSource>>,
) -> (StageSender, PluginStage) {
    let (commands, receiver) = sync_channel(COMMAND_CAPACITY);
    (
        StageSender { commands },
        PluginStage {
            plugin_id: plugin_id.into(),
            stage,
            commands: receiver,
            input,
            held: Vec::with_capacity(MAX_HELD),
            next_note_id: 0,
            events: Vec::with_capacity(COMMAND_CAPACITY * 2),
            in_buf: vec![0.0; BLOCK_FRAMES as usize],
            failed: false,
        },
    )
}

impl PluginStage {
    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    fn release_where(&mut self, keep: impl Fn(u64) -> bool) {
        let events = &mut self.events;
        self.held.retain(|&(id, note_id, key)| {
            if keep(id) {
                true
            } else {
                events.push(Event::NoteOff {
                    frame: 0,
                    note_id,
                    key,
                });
                false
            }
        });
    }

    fn apply(&mut self, command: StageCommand) {
        match command {
            StageCommand::NoteOn { id, key, velocity } => {
                self.release_where(|held| held != id);
                if self.held.len() >= MAX_HELD {
                    let (_, note_id, key) = self.held.remove(0);
                    self.events.push(Event::NoteOff {
                        frame: 0,
                        note_id,
                        key,
                    });
                }
                let note_id = self.next_note_id;
                self.next_note_id = self.next_note_id.wrapping_add(1) & i32::MAX;
                self.held.push((id, note_id, key));
                self.events.push(Event::NoteOn {
                    frame: 0,
                    note_id,
                    key,
                    velocity,
                });
            }
            StageCommand::NoteOff { id } => self.release_where(|held| held != id),
            StageCommand::AllOff => self.release_where(|_| false),
        }
    }
}

impl BlockSource for PluginStage {
    fn render_block(&mut self, out: &mut [f32]) {
        self.events.clear();
        while let Ok(command) = self.commands.try_recv() {
            self.apply(command);
        }
        sort_events(&mut self.events);
        if self.failed {
            out.fill(0.0);
            return;
        }
        for (index, chunk) in out.chunks_mut(BLOCK_FRAMES as usize).enumerate() {
            let input = match &mut self.input {
                Some(source) => {
                    let buf = &mut self.in_buf[..chunk.len()];
                    source.render_block(buf);
                    Some(&*buf)
                }
                None => None,
            };
            let events: &[Event] = if index == 0 { &self.events } else { &[] };
            if let Err(error) = self.stage.process(input, chunk, events) {
                eprintln!("aura: plugin `{}` stopped: {error}", self.plugin_id);
                self.failed = true;
                chunk.fill(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelLayout, PluginHostError, Processor, Tail};

    /// Outputs held-note count plus input.
    struct Counter {
        held: i32,
    }

    impl Processor for Counter {
        fn layout(&self) -> ChannelLayout {
            ChannelLayout {
                inputs: 1,
                outputs: 1,
            }
        }
        fn latency_frames(&self) -> u32 {
            0
        }
        fn tail(&mut self) -> Tail {
            Tail::Frames(0)
        }
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [&mut [f32]],
            events: &[Event],
        ) -> Result<(), PluginHostError> {
            for event in events {
                match event {
                    Event::NoteOn { .. } => self.held += 1,
                    Event::NoteOff { .. } => self.held -= 1,
                }
            }
            for (out, input) in outputs[0].iter_mut().zip(inputs[0]) {
                *out = self.held as f32 + input;
            }
            Ok(())
        }
    }

    fn stage(input: Option<Box<dyn BlockSource>>) -> (StageSender, PluginStage) {
        plugin_stage(
            "counter",
            MonoStage::new(Box::new(Counter { held: 0 }), BLOCK_FRAMES as usize),
            input,
        )
    }

    #[test]
    fn notes_are_shared_by_one_instance() {
        let (sender, mut stage) = stage(None);
        let mut block = vec![0.0; 1000];
        sender
            .send(StageCommand::NoteOn {
                id: 1,
                key: 60,
                velocity: 1.0,
            })
            .unwrap();
        sender
            .send(StageCommand::NoteOn {
                id: 2,
                key: 64,
                velocity: 1.0,
            })
            .unwrap();
        stage.render_block(&mut block);
        assert!(block.iter().all(|s| *s == 2.0));

        sender.send(StageCommand::NoteOff { id: 1 }).unwrap();
        stage.render_block(&mut block);
        assert!(block.iter().all(|s| *s == 1.0));

        sender.send(StageCommand::AllOff).unwrap();
        stage.render_block(&mut block);
        assert!(block.iter().all(|s| *s == 0.0));
        assert_eq!(stage.held_count(), 0);
    }

    #[test]
    fn retriggering_an_id_releases_it_first() {
        let (sender, mut stage) = stage(None);
        let mut block = vec![0.0; 10];
        for _ in 0..2 {
            sender
                .send(StageCommand::NoteOn {
                    id: 7,
                    key: 60,
                    velocity: 1.0,
                })
                .unwrap();
        }
        stage.render_block(&mut block);
        assert_eq!(block[0], 1.0);
    }

    struct Half;
    impl BlockSource for Half {
        fn render_block(&mut self, out: &mut [f32]) {
            out.fill(0.5);
        }
    }

    #[test]
    fn effect_stage_processes_its_input_source() {
        let (_sender, mut stage) = stage(Some(Box::new(Half)));
        let mut block = vec![0.0; 600];
        stage.render_block(&mut block);
        assert!(block.iter().all(|s| *s == 0.5));
    }
}
