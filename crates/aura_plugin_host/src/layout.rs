//! A plugin seen through Aura's audio layouts: plugins have any number of channels, Aura signals
//! are mono or stereo.

use crate::{Event, PluginHostError, Processor};
use aura_render::Layout;

/// A [`Processor`] adapted to a mono or stereo [`Layout`].
///
/// - **Input.** A mono input feeds every plugin input channel. A stereo input feeds the first
///   two (left, right), or their average when the plugin has one input; further plugin inputs
///   are silent.
/// - **Output.** Mono is the average of the plugin's output channels. Stereo is its first two
///   outputs, a single output duplicated, or silence without outputs.
pub struct LayoutStage {
    processor: Box<dyn Processor>,
    layout: Layout,
    inputs: Vec<Vec<f32>>,
    outputs: Vec<Vec<f32>>,
}

impl LayoutStage {
    /// `max_block` is the most frames any [`LayoutStage::process`] call will pass.
    pub fn new(processor: Box<dyn Processor>, max_block: usize, layout: Layout) -> Self {
        let channels = processor.layout();
        Self {
            processor,
            layout,
            inputs: vec![vec![0.0; max_block]; channels.inputs],
            outputs: vec![vec![0.0; max_block]; channels.outputs],
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn processor_mut(&mut self) -> &mut dyn Processor {
        self.processor.as_mut()
    }

    pub fn latency_frames(&self) -> u32 {
        self.processor.latency_frames()
    }

    /// Processes `frames` frames. `input`, when given, and `out` hold one slice per channel of
    /// the stage's layout, each at least `frames` long.
    pub fn process(
        &mut self,
        input: Option<&[&[f32]]>,
        out: &mut [&mut [f32]],
        frames: usize,
        events: &[Event],
    ) -> Result<(), PluginHostError> {
        let plugin_inputs = self.inputs.len();
        for (index, channel) in self.inputs.iter_mut().enumerate() {
            let channel = &mut channel[..frames];
            match input {
                None | Some([]) => channel.fill(0.0),
                Some([mono]) => channel.copy_from_slice(&mono[..frames]),
                Some([left, right, ..]) if plugin_inputs == 1 => {
                    for (sample, (l, r)) in channel.iter_mut().zip(left.iter().zip(right.iter())) {
                        *sample = (l + r) * 0.5;
                    }
                }
                Some(channels) => match channels.get(index) {
                    Some(source) => channel.copy_from_slice(&source[..frames]),
                    None => channel.fill(0.0),
                },
            }
        }
        let inputs: Vec<&[f32]> = self.inputs.iter().map(|c| &c[..frames]).collect();
        let mut outputs: Vec<&mut [f32]> =
            self.outputs.iter_mut().map(|c| &mut c[..frames]).collect();
        self.processor.process(&inputs, &mut outputs, events)?;

        match (self.layout, self.outputs.as_slice()) {
            (_, []) => {
                for channel in out.iter_mut() {
                    channel[..frames].fill(0.0);
                }
            }
            (Layout::Mono, outputs) => {
                let target = &mut out[0][..frames];
                target.fill(0.0);
                let scale = 1.0 / outputs.len() as f32;
                for channel in outputs {
                    for (sum, sample) in target.iter_mut().zip(&channel[..frames]) {
                        *sum += sample * scale;
                    }
                }
            }
            (Layout::Stereo, [only]) => {
                for channel in out.iter_mut() {
                    channel[..frames].copy_from_slice(&only[..frames]);
                }
            }
            (Layout::Stereo, [left, right, ..]) => {
                out[0][..frames].copy_from_slice(&left[..frames]);
                out[1][..frames].copy_from_slice(&right[..frames]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelLayout, Tail};

    /// Two outputs: input channel 0 + 1, and input channel 1 (or 0) − 1.
    struct Split {
        inputs: usize,
    }

    impl Processor for Split {
        fn layout(&self) -> ChannelLayout {
            ChannelLayout {
                inputs: self.inputs,
                outputs: 2,
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
            _events: &[Event],
        ) -> Result<(), PluginHostError> {
            let second = inputs.len().min(2) - 1;
            for frame in 0..outputs[0].len() {
                outputs[0][frame] = inputs[0][frame] + 1.0;
                outputs[1][frame] = inputs[second][frame] - 1.0;
            }
            Ok(())
        }
    }

    fn run(stage: &mut LayoutStage, input: &[&[f32]]) -> Vec<Vec<f32>> {
        let mut out = vec![vec![0.0; 2]; stage.layout().channels()];
        let mut slices: Vec<&mut [f32]> = out.iter_mut().map(Vec::as_mut_slice).collect();
        stage.process(Some(input), &mut slices, 2, &[]).expect("process");
        out
    }

    #[test]
    fn stereo_keeps_channels_apart() {
        let mut stage = LayoutStage::new(Box::new(Split { inputs: 2 }), 2, Layout::Stereo);
        let out = run(&mut stage, &[&[0.5, 0.5], &[0.25, 0.25]]);
        assert_eq!(out, vec![vec![1.5, 1.5], vec![-0.75, -0.75]]);
    }

    #[test]
    fn mono_feeds_every_input_and_averages_outputs() {
        let mut stage = LayoutStage::new(Box::new(Split { inputs: 2 }), 2, Layout::Mono);
        let out = run(&mut stage, &[&[0.5, 0.5]]);
        assert_eq!(out, vec![vec![0.5, 0.5]]);
    }

    #[test]
    fn stereo_into_a_mono_input_plugin_is_averaged() {
        let mut stage = LayoutStage::new(Box::new(Split { inputs: 1 }), 2, Layout::Stereo);
        let out = run(&mut stage, &[&[1.0, 1.0], &[0.0, 0.0]]);
        assert_eq!(out, vec![vec![1.5, 1.5], vec![-0.5, -0.5]]);
    }
}
