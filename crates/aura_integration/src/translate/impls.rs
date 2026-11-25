//! Node type implementations: registration and the mono/stereo × deterministic/streaming matrix.
//!
//! Each node type registers the implementations it provides ([`Registration`]). When the
//! registry is built, [`Registration::finalize`] fills every cell it left empty, so translation
//! reads one cell and never falls back at translate time:
//!
//! | | deterministic | streaming |
//! |---|---|---|
//! | mono | required | defaults to mono deterministic |
//! | stereo | defaults to dual mono (mono deterministic) | defaults to stereo deterministic when given, else dual mono (mono streaming) |
//!
//! Only layout-polymorphic node types (`A: Audio`) have a stereo row; fixed-layout node types
//! have the mono row only. Design: `docs/design/audio-layouts.md`.

use super::streaming::NodeCtx;
use super::{NodeValue, TranslateError};
use aura_imp::NodeId;
use aura_render::{
    ChannelView, Layout, Sampler, SharedSampler, StereoPair, StereoSampler, LEFT, RIGHT,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

/// One implementation: builds a node's runtime value at translate time.
pub(crate) type Build =
    Arc<dyn Fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> + Send + Sync>;

/// Execution mode a cell targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Deterministic,
    Streaming,
}

struct Modes {
    deterministic: Build,
    streaming: Option<Build>,
}

/// The implementations a node type provides.
pub(crate) struct Registration {
    mono: Modes,
    /// `Some` for layout-polymorphic node types; the stereo functions it provides.
    stereo: Option<Option<Modes>>,
}

fn build(f: impl Fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> + Send + Sync + 'static) -> Build {
    Arc::new(f)
}

impl Registration {
    /// A fixed-layout node type with its deterministic implementation.
    pub(crate) fn fixed(
        deterministic: impl Fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            mono: Modes {
                deterministic: build(deterministic),
                streaming: None,
            },
            stereo: None,
        }
    }

    /// A layout-polymorphic node type (`A: Audio`) with its mono deterministic implementation.
    pub(crate) fn layout(
        mono: impl Fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            stereo: Some(None),
            ..Self::fixed(mono)
        }
    }

    /// The mono (or fixed-layout) streaming implementation.
    pub(crate) fn streaming(
        mut self,
        streaming: impl Fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.mono.streaming = Some(build(streaming));
        self
    }

    /// A true stereo deterministic implementation instead of dual mono.
    pub(crate) fn stereo(
        mut self,
        deterministic: impl Fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        assert!(self.stereo.is_some(), "stereo implementation on a fixed-layout node type");
        self.stereo = Some(Some(Modes {
            deterministic: build(deterministic),
            streaming: None,
        }));
        self
    }

    /// A true stereo streaming implementation; requires [`Registration::stereo`].
    pub(crate) fn stereo_streaming(
        mut self,
        streaming: impl Fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        let Some(Some(modes)) = self.stereo.as_mut() else {
            panic!("stereo streaming implementation without a stereo implementation");
        };
        modes.streaming = Some(build(streaming));
        self
    }

    /// Fills every empty cell of the matrix.
    pub(crate) fn finalize(self) -> Matrix {
        let mono_deterministic = self.mono.deterministic;
        let mono_streaming = self
            .mono
            .streaming
            .unwrap_or_else(|| mono_deterministic.clone());
        let stereo = self.stereo.map(|stereo| match stereo {
            Some(modes) => {
                let streaming = modes
                    .streaming
                    .unwrap_or_else(|| modes.deterministic.clone());
                [modes.deterministic, streaming]
            }
            None => [
                dual_mono(mono_deterministic.clone()),
                dual_mono(mono_streaming.clone()),
            ],
        });
        Matrix {
            mono: [mono_deterministic, mono_streaming],
            stereo,
        }
    }
}

/// A node type's finalized implementations.
pub(crate) struct Matrix {
    mono: [Build; 2],
    stereo: Option<[Build; 2]>,
}

impl Matrix {
    pub(crate) fn is_layout_polymorphic(&self) -> bool {
        self.stereo.is_some()
    }

    /// The implementation for a layout and mode. Fixed-layout node types ignore `layout`.
    pub(crate) fn cell(&self, layout: Layout, mode: Mode) -> &Build {
        let row = match (layout, &self.stereo) {
            (Layout::Stereo, Some(stereo)) => stereo,
            _ => &self.mono,
        };
        &row[mode as usize]
    }
}

/// Every Aura node type's finalized implementations, built once.
pub(crate) fn node_impls() -> &'static BTreeMap<&'static str, Matrix> {
    static IMPLS: OnceLock<BTreeMap<&'static str, Matrix>> = OnceLock::new();
    IMPLS.get_or_init(|| {
        super::nodes::registrations()
            .into_iter()
            .map(|(id, registration)| (id, registration.finalize()))
            .collect()
    })
}

/// A translated input shared between the two channel passes of a dual-mono node.
#[derive(Clone)]
pub(crate) enum SharedInput {
    Mono(Arc<dyn Sampler>),
    Stereo(Arc<dyn StereoSampler>),
}

/// One channel pass of a dual-mono node: which node, which channel, and its inputs translated
/// so far (by port and edge index), so the second pass reuses them instead of translating
/// upstream again.
#[derive(Clone)]
pub(crate) struct ChannelPass {
    pub(crate) consumer: NodeId,
    pub(crate) channel: usize,
    memo: Rc<RefCell<BTreeMap<(String, usize), SharedInput>>>,
}

impl ChannelPass {
    pub(crate) fn remembered(&self, port: &str, index: usize) -> Option<SharedInput> {
        self.memo.borrow().get(&(port.to_string(), index)).cloned()
    }

    pub(crate) fn remember(&self, port: &str, index: usize, input: SharedInput) {
        self.memo.borrow_mut().insert((port.to_string(), index), input);
    }

    /// The input as this pass sees it: a mono input shared, a stereo input as this channel.
    pub(crate) fn view(&self, input: SharedInput) -> Box<dyn Sampler> {
        match input {
            SharedInput::Mono(sampler) => Box::new(SharedSampler(sampler)),
            SharedInput::Stereo(sampler) => Box::new(ChannelView::new(sampler, self.channel)),
        }
    }
}

/// Stereo from a mono implementation: run it once per channel, each pass seeing that channel of
/// every stereo input (and the same mono inputs), and pair the results.
fn dual_mono(mono: Build) -> Build {
    Arc::new(move |ctx: &mut NodeCtx<'_, '_>| {
        let memo = Rc::new(RefCell::new(BTreeMap::new()));
        let channel = |ctx: &mut NodeCtx<'_, '_>, channel: usize| {
            let pass = ChannelPass {
                consumer: ctx.node.id.clone(),
                channel,
                memo: Rc::clone(&memo),
            };
            ctx.bindings.channel_passes.borrow_mut().push(pass);
            let layout = std::mem::replace(&mut ctx.layout, Layout::Mono);
            let result = mono(ctx);
            ctx.layout = layout;
            ctx.bindings.channel_passes.borrow_mut().pop();
            match result? {
                NodeValue::Signal(sampler) => Ok(sampler),
                _ => Err(ctx.missing_port()),
            }
        };
        let left = channel(ctx, LEFT)?;
        let right = channel(ctx, RIGHT)?;
        Ok(NodeValue::Stereo(Box::new(StereoPair::new(left, right))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_node_type_has_a_complete_matrix() {
        let impls = node_impls();
        assert!(impls.contains_key("multiply"));
        for (id, matrix) in impls {
            for mode in [Mode::Deterministic, Mode::Streaming] {
                let _ = matrix.cell(Layout::Mono, mode);
                let _ = matrix.cell(Layout::Stereo, mode);
            }
            if matrix.is_layout_polymorphic() {
                assert!(
                    !Arc::ptr_eq(
                        matrix.cell(Layout::Mono, Mode::Deterministic),
                        matrix.cell(Layout::Stereo, Mode::Deterministic)
                    ),
                    "{id}: stereo must not reuse the mono cell"
                );
            }
        }
    }
}
