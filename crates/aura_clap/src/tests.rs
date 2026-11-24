use super::*;
use aura_clap_fixture::{gain_state, FixtureEntry, GAIN_ID, SINE_ID};
use aura_plugin_host::{
    EventSource, MonoStage, NoteEventSource, ProcessorConfig, RenderCache, RenderMode,
    BLOCK_FRAMES,
};
use aura_render::Sampler;
use aura_sample::SampleRate;
use aura_sequencing::{NoteEvent, Score, ScoreSignal, Semitone, Tempo, TimeSignature};

const RATE: SampleRate = SampleRate::RATE_44100;

fn fixture_host() -> ClapHost {
    let host = ClapHost::new(Vec::new());
    let entry = PluginEntry::load_from_clack::<FixtureEntry>(c"/fixture/aura_clap_fixture.clap")
        .expect("load fixture entry");
    host.add_entry(entry, "/fixture/aura_clap_fixture.clap");
    host
}

fn config(mode: RenderMode) -> ProcessorConfig {
    ProcessorConfig {
        sample_rate: RATE,
        max_block_frames: BLOCK_FRAMES,
        mode,
    }
}

fn stage(host: &ClapHost, id: &str, state: Option<&[u8]>) -> MonoStage {
    let factory = host.resolve(id, state).expect("resolve");
    let processor = factory.create(config(RenderMode::Offline)).expect("create");
    MonoStage::new(processor, BLOCK_FRAMES as usize)
}

/// 120 bpm: A4 from 0.5 s to 1.0 s.
fn score() -> ScoreSignal {
    ScoreSignal::finite(Score::new(
        TimeSignature::default(),
        Tempo::new(120.0).unwrap(),
        vec![NoteEvent::new(Semitone(69), 1.0, 1.0)],
    ))
}

fn sine_cache(host: &ClapHost) -> RenderCache {
    RenderCache::new(
        SINE_ID,
        stage(host, SINE_ID, None),
        EventSource::Score(NoteEventSource::from_score_signal(&score(), RATE)),
        None,
        RATE,
    )
}

fn frame(cache: &RenderCache, frame: u64) -> f32 {
    cache.at(frame as f64 / 44100.0)
}

#[test]
fn discovers_both_fixture_plugins() {
    let discovery = fixture_host().discover();
    let ids: Vec<&str> = discovery.plugins.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec![SINE_ID, GAIN_ID]);
    assert_eq!(discovery.plugins[0].name, "Aura Fixture Sine");
    assert!(discovery.plugins[0].features.iter().any(|f| f == "instrument"));
    assert!(discovery.failures.is_empty());
}

#[test]
fn unknown_plugin_is_not_found() {
    let error = fixture_host()
        .resolve("com.example.missing", None)
        .err()
        .expect("missing plugin");
    assert_eq!(
        error,
        PluginHostError::NotFound {
            id: "com.example.missing".into()
        }
    );
}

#[test]
fn sine_onset_lands_on_the_score_frame_with_latency_compensated() {
    let host = fixture_host();
    let cache = sine_cache(&host);
    // Onset at frame 22050; the sine starts at phase 0, so its first non-zero sample is next.
    for f in [0, 1000, 22049, 22050] {
        assert_eq!(frame(&cache, f), 0.0, "silent at {f}");
    }
    assert!(frame(&cache, 22051).abs() > 0.0);
    let peak = (22050..44100).map(|f| frame(&cache, f).abs()).fold(0.0, f32::max);
    assert!((peak - 0.25).abs() < 0.01, "peak {peak}");
    // Released at 1.0 s with a 0.1 s release: silent by 1.1 s.
    assert_eq!(frame(&cache, 44100 + 4500), 0.0);
    assert!(cache.failure().is_none());
}

#[test]
fn renders_are_reproducible_and_seekable() {
    let host = fixture_host();
    let sequential = sine_cache(&host);
    let expected: Vec<f32> = (0..50_000).map(|f| frame(&sequential, f)).collect();
    let again = sine_cache(&host);
    for f in (0..50_000u64).rev().step_by(331) {
        assert_eq!(frame(&again, f).to_bits(), expected[f as usize].to_bits());
    }
}

struct Constant(f32);
impl Sampler for Constant {
    fn at(&self, _t: f64) -> f32 {
        self.0
    }
}

#[test]
fn gain_effect_applies_its_stored_state() {
    let host = fixture_host();
    let effect = |state: Option<Vec<u8>>| {
        RenderCache::new(
            GAIN_ID,
            stage(&host, GAIN_ID, state.as_deref()),
            EventSource::None,
            Some(Box::new(Constant(0.8))),
            RATE,
        )
    };
    assert_eq!(frame(&effect(None), 100), 0.8 * 0.5);
    assert_eq!(frame(&effect(Some(gain_state(0.25))), 100), 0.8 * 0.25);
}

#[test]
fn state_for_a_plugin_without_state_support_is_rejected() {
    let host = fixture_host();
    let factory = host.resolve(SINE_ID, Some(&[1, 2, 3])).expect("resolve");
    let error = factory.create(config(RenderMode::Offline)).err().expect("error");
    assert_eq!(error, PluginHostError::StateLoad { id: SINE_ID.into() });
}

#[test]
fn invalid_state_is_rejected() {
    let host = fixture_host();
    let factory = host.resolve(GAIN_ID, Some(&[1])).expect("resolve");
    let error = factory.create(config(RenderMode::Offline)).err().expect("error");
    assert_eq!(error, PluginHostError::StateLoad { id: GAIN_ID.into() });
}

#[test]
fn instances_on_other_threads_share_the_main_thread() {
    let host = Arc::new(fixture_host());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let host = Arc::clone(&host);
            std::thread::spawn(move || {
                let cache = sine_cache(&host);
                frame(&cache, 30_000)
            })
        })
        .collect();
    let values: Vec<f32> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(values.windows(2).all(|w| w[0] == w[1]));
}
