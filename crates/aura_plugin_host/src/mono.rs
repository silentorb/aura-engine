//! Aura is mono end to end; plugins are usually stereo.

use crate::{Event, PluginHostError, Processor};

/// A [`Processor`] seen as mono: a mono input is copied to every input channel, and the
/// output is the average of the output channels.
pub struct MonoStage {
    processor: Box<dyn Processor>,
    inputs: Vec<Vec<f32>>,
    outputs: Vec<Vec<f32>>,
}

impl MonoStage {
    /// `max_block` is the most frames any [`MonoStage::process`] call will pass.
    pub fn new(processor: Box<dyn Processor>, max_block: usize) -> Self {
        let layout = processor.layout();
        Self {
            processor,
            inputs: vec![vec![0.0; max_block]; layout.inputs],
            outputs: vec![vec![0.0; max_block]; layout.outputs],
        }
    }

    pub fn processor_mut(&mut self) -> &mut dyn Processor {
        self.processor.as_mut()
    }

    pub fn latency_frames(&self) -> u32 {
        self.processor.latency_frames()
    }

    /// Processes `out.len()` frames. `input`, when given, has the same length.
    pub fn process(
        &mut self,
        input: Option<&[f32]>,
        out: &mut [f32],
        events: &[Event],
    ) -> Result<(), PluginHostError> {
        let frames = out.len();
        for channel in &mut self.inputs {
            match input {
                Some(input) => channel[..frames].copy_from_slice(&input[..frames]),
                None => channel[..frames].fill(0.0),
            }
        }
        let inputs: Vec<&[f32]> = self.inputs.iter().map(|c| &c[..frames]).collect();
        let mut outputs: Vec<&mut [f32]> =
            self.outputs.iter_mut().map(|c| &mut c[..frames]).collect();
        self.processor.process(&inputs, &mut outputs, events)?;

        out.fill(0.0);
        if self.outputs.is_empty() {
            return Ok(());
        }
        let scale = 1.0 / self.outputs.len() as f32;
        for channel in &self.outputs {
            for (sum, sample) in out.iter_mut().zip(&channel[..frames]) {
                *sum += sample * scale;
            }
        }
        Ok(())
    }
}
