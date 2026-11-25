//! Aura's node types: their deterministic implementations, and every node type's registration.
//!
//! Streaming implementations live in [`super::streaming`]; plugin nodes in [`super::plugin`].
//! [`registrations`] lists each node type once with all the implementations it provides; the
//! registry fills in the rest of its matrix (see [`super::impls`]).

use super::impls::Registration;
use super::note_schedule::NoteSchedule;
use super::streaming::{self, NodeCtx};
use super::*;
use aura_imp::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT};
use aura_render::{StereoChannel, StereoPair, StereoToMono, LEFT, RIGHT};

/// Calls a translator helper that takes the shared translation arguments first.
macro_rules! with_ctx {
    ($ctx:expr, $helper:path $(, $arg:expr)* $(,)?) => {
        $helper(
            $ctx.graph,
            $ctx.registry,
            $ctx.sample_rate,
            $ctx.edges_by_target,
            $ctx.visiting,
            $ctx.bindings,
            $($arg),*
        )
    };
}

fn signal(signal: impl Signal + 'static) -> Result<NodeValue, TranslateError> {
    Ok(NodeValue::Signal(Box::new(signal)))
}

fn notation(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let node_id = ctx.node.id.as_str();
    match ctx.bindings.scores.get(node_id) {
        Some(score) if ctx.output_port == "score" => Ok(NodeValue::ScoreSignal(score.clone())),
        Some(_) => Err(ctx.missing_port()),
        None => Err(TranslateError::MissingScoreBinding {
            node: node_id.into(),
        }),
    }
}

/// `output` passes its input through unchanged, whatever its layout.
fn output(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let source = wired_source(ctx.edges_by_target, &ctx.node.id, "value")?;
    with_ctx!(ctx, translate_node, &source.node, &source.port)
}

fn parameter(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let node_id = ctx.node.id.as_str();
    let value = ctx
        .node
        .inputs
        .get("value")
        .ok_or_else(|| TranslateError::UnwiredInput {
            node: node_id.into(),
            port: "value".into(),
        })?;
    signal(Constant(primitive_to_f64(node_id, "value", value)? as f32))
}

fn time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    signal(TimeSignal { rate: ctx.rate() })
}

fn time_elapsed(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let start = ctx.input("start")?;
    let _time = ctx.signal("time")?;
    signal(TimeElapsedSignal {
        start,
        rate: ctx.rate(),
    })
}

fn sine_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let frequency = ctx.input("frequency")?;
    let time = ctx.input("time")?;
    signal(SineSignal { frequency, time })
}

/// Mono gain; stereo is dual mono.
fn multiply_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let a = ctx.input("a")?;
    let b = ctx.input("b")?;
    signal(MultiplySignal { a, b })
}

fn multiply_control(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let a = ctx.input("a")?;
    let b = ctx.input("b")?;
    signal(MultiplySignal { a, b })
}

/// Mono sum; stereo is dual mono.
fn add_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let inputs = ctx.signals("inputs")?.into_iter().map(Input::new).collect();
    signal(AddSignal { inputs })
}

fn exponential_sweep_sine_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let start_hz = ctx.input("start_hz")?;
    let decay_rate = ctx.input("decay_rate")?;
    let elapsed = ctx.input("elapsed")?;
    signal(ExponentialSweepSineSignal {
        start_hz,
        decay_rate,
        elapsed,
    })
}

fn deterministic_noise_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let seed = ctx.input("seed")?;
    let time = ctx.input("time")?;
    signal(DeterministicNoiseSignal { seed, time })
}

fn highpass_noise_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let seed = ctx.input("seed")?;
    let time = ctx.input("time")?;
    let dt = ctx.input("dt")?;
    signal(HighpassNoiseSignal { seed, time, dt })
}

fn bandlimited_saw_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let frequency = ctx.input("frequency")?;
    let elapsed = ctx.input("elapsed")?;
    let cutoff = ctx.input("cutoff")?;
    signal(BandlimitedSawSignal {
        frequency,
        elapsed,
        cutoff,
    })
}

fn linear_adsr(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let attack = ctx.input("attack")?;
    let decay = ctx.input("decay")?;
    let sustain = ctx.input("sustain")?;
    let release = ctx.input("release")?;
    let elapsed = ctx.input("elapsed")?;
    let note_duration = ctx.input("note_duration")?;
    signal(LinearAdsrSignal {
        attack,
        decay,
        sustain,
        release,
        elapsed,
        note_duration,
    })
}

fn semitone_to_hz(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let semitone = ctx.input("semitone")?;
    signal(SemitoneToHzSignal { semitone })
}

fn epic_minor_progression_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let node = ctx.node;
    Ok(NodeValue::ProgressionSignal(ProgressionSignal::finite(
        with_ctx!(ctx, build_epic_minor_progression, node)?,
    )))
}

fn constant_tempo(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    Ok(NodeValue::Tempo(build_constant_tempo(ctx.node)?))
}

fn constant_time_signature(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    Ok(NodeValue::TimeSignature(build_constant_time_signature(
        ctx.node,
    )?))
}

fn tempo_at_time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let (node, port) = (ctx.node, ctx.output_port);
    with_ctx!(ctx, translate_tempo_at_time_port, &node.id, port)
}

fn time_signature_at_time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let (node, port) = (ctx.node, ctx.output_port);
    with_ctx!(ctx, translate_time_signature_at_time_port, &node.id, port)
}

fn arpeggio_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let node = ctx.node;
    Ok(NodeValue::ScoreSignal(ScoreSignal::finite(with_ctx!(
        ctx,
        build_arpeggio_score,
        node
    )?)))
}

fn bass_line_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let node = ctx.node;
    Ok(NodeValue::ScoreSignal(ScoreSignal::finite(with_ctx!(
        ctx,
        build_bass_line_score,
        node
    )?)))
}

fn drum_grid_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let node = ctx.node;
    Ok(NodeValue::ScoreSignal(ScoreSignal::finite(with_ctx!(
        ctx,
        build_drum_grid_score,
        node
    )?)))
}

fn loop_node(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let node = ctx.node;
    with_ctx!(ctx, translate_loop_value, &node.id)
}

fn note_at_time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let (node, port) = (ctx.node, ctx.output_port);
    with_ctx!(ctx, translate_note_at_time_port, &node.id, port)
}

fn note_param_at_time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let (node, port) = (ctx.node, ctx.output_port);
    with_ctx!(ctx, translate_note_param_at_time_port, node, port)
}

fn chord_at_time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let (node, port) = (ctx.node, ctx.output_port);
    with_ctx!(ctx, translate_chord_at_time_port, &node.id, port)
}

fn note_envelope(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let score = ctx.score("score")?;
    signal(NoteEnvelopeSignal {
        schedule: NoteSchedule::from_score_signal(&score, ctx.sample_rate),
        adsr: LinearAdsr::default(),
        rate: ctx.rate(),
    })
}

fn poly_sine_score(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let score = ctx.score("score")?;
    let _time = ctx.signal("time")?;
    signal(PolySineScoreSignal {
        schedule: NoteSchedule::from_score_signal(&score, ctx.sample_rate),
        adsr: LinearAdsr::default(),
        rate: ctx.rate(),
    })
}

fn pan(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let audio = ctx.input("audio")?;
    let position = ctx.input("position")?;
    Ok(NodeValue::Stereo(Box::new(PanSignal { audio, position })))
}

fn stereo_to_mono(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    signal(StereoToMono::new(ctx.stereo_signal("audio")?))
}

fn stereo_join(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let left = ctx.signal("left")?;
    let right = ctx.signal("right")?;
    Ok(NodeValue::Stereo(Box::new(StereoPair::new(left, right))))
}

/// One channel of a stereo signal, by output port (`left` or `right`).
fn stereo_split(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let channel = match ctx.output_port {
        "left" => LEFT,
        "right" => RIGHT,
        _ => return Err(ctx.missing_port()),
    };
    signal(StereoChannel::new(ctx.stereo_signal("audio")?, channel))
}

/// Every Aura node type with the implementations it provides.
pub(super) fn registrations() -> Vec<(&'static str, Registration)> {
    use Registration as R;
    vec![
        (
            NOTATION_NODE_TYPE,
            R::fixed(notation).streaming(streaming::stream_notation),
        ),
        ("output", R::fixed(output)),
        ("parameter", R::fixed(parameter)),
        ("time", R::fixed(time)),
        ("time_elapsed", R::fixed(time_elapsed)),
        ("sine", R::fixed(sine_node)),
        ("multiply", R::layout(multiply_node)),
        ("multiply_control", R::fixed(multiply_control)),
        ("add", R::layout(add_node)),
        ("pan", R::fixed(pan)),
        ("stereo_to_mono", R::fixed(stereo_to_mono)),
        ("stereo_join", R::fixed(stereo_join)),
        ("stereo_split", R::fixed(stereo_split)),
        ("exponential_sweep_sine", R::fixed(exponential_sweep_sine_node)),
        ("deterministic_noise", R::fixed(deterministic_noise_node)),
        ("highpass_noise", R::fixed(highpass_noise_node)),
        ("bandlimited_saw", R::fixed(bandlimited_saw_node)),
        (
            "linear_adsr",
            R::fixed(linear_adsr).streaming(streaming::stream_linear_adsr),
        ),
        ("semitone_to_hz", R::fixed(semitone_to_hz)),
        ("epic_minor_progression", R::fixed(epic_minor_progression_node)),
        ("constant_tempo", R::fixed(constant_tempo)),
        ("constant_time_signature", R::fixed(constant_time_signature)),
        ("tempo_at_time", R::fixed(tempo_at_time)),
        ("time_signature_at_time", R::fixed(time_signature_at_time)),
        ("arpeggio", R::fixed(arpeggio_node)),
        ("bass_line", R::fixed(bass_line_node)),
        ("drum_grid", R::fixed(drum_grid_node)),
        ("loop", R::fixed(loop_node)),
        (
            "note_at_time",
            R::fixed(note_at_time).streaming(streaming::stream_note_at_time),
        ),
        (
            "note_param_at_time",
            R::fixed(note_param_at_time).streaming(streaming::stream_note_param_at_time),
        ),
        ("chord_at_time", R::fixed(chord_at_time)),
        (
            "note_envelope",
            R::fixed(note_envelope).streaming(streaming::stream_note_envelope),
        ),
        (
            "poly_sine_score",
            R::fixed(poly_sine_score).streaming(streaming::stream_poly_sine_score),
        ),
        (
            PLUGIN_INSTRUMENT,
            R::fixed(plugin::translate_plugin_deterministic).streaming(streaming::stream_plugin),
        ),
        // One plugin instance processes both channels: a true stereo implementation, never dual
        // mono. The same functions read the resolved layout from the context.
        (
            PLUGIN_EFFECT,
            R::layout(plugin::translate_plugin_deterministic)
                .streaming(streaming::stream_plugin)
                .stereo(plugin::translate_plugin_deterministic)
                .stereo_streaming(streaming::stream_plugin),
        ),
    ]
}
