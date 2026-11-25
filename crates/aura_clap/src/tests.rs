use super::*;
use aura_clap_fixture::{gain_state, FixtureEntry, GAIN_ID, SINE_ID, SINE_LEVEL_PARAM};
use aura_plugin_host::{
    EventSource, LayoutStage, NoteEventSource, ParamSetting, PluginSettings, ProcessorConfig,
    RenderCache, RenderMode, BLOCK_FRAMES,
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

fn settings(state: Option<&[u8]>) -> PluginSettings {
    PluginSettings {
        state: state.map(<[u8]>::to_vec),
        params: Vec::new(),
    }
}

fn stage(host: &ClapHost, id: &str, settings: &PluginSettings) -> LayoutStage {
    let factory = host.resolve(id, settings).expect("resolve");
    let processor = factory.create(config(RenderMode::Offline)).expect("create");
    LayoutStage::new(processor, BLOCK_FRAMES as usize, aura_render::Layout::Mono)
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
    sine_cache_with(host, &PluginSettings::default())
}

fn sine_cache_with(host: &ClapHost, settings: &PluginSettings) -> RenderCache {
    RenderCache::new(
        SINE_ID,
        stage(host, SINE_ID, settings),
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
        .resolve("com.example.missing", &PluginSettings::default())
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
            stage(&host, GAIN_ID, &settings(state.as_deref())),
            EventSource::None,
            Some(aura_render::AudioSampler::Mono(Box::new(Constant(0.8)))),
            RATE,
        )
    };
    assert_eq!(frame(&effect(None), 100), 0.8 * 0.5);
    assert_eq!(frame(&effect(Some(gain_state(0.25))), 100), 0.8 * 0.25);
}

#[test]
fn state_for_a_plugin_without_state_support_is_rejected() {
    let host = fixture_host();
    let factory = host.resolve(SINE_ID, &settings(Some(&[1, 2, 3]))).expect("resolve");
    let error = factory.create(config(RenderMode::Offline)).err().expect("error");
    assert_eq!(error, PluginHostError::StateLoad { id: SINE_ID.into() });
}

#[test]
fn invalid_state_is_rejected() {
    let host = fixture_host();
    let factory = host.resolve(GAIN_ID, &settings(Some(&[1]))).expect("resolve");
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

fn level(value: f64) -> PluginSettings {
    PluginSettings {
        state: None,
        params: vec![ParamSetting {
            id: SINE_LEVEL_PARAM,
            value,
        }],
    }
}

fn note_peak(cache: &RenderCache) -> f32 {
    (22050..44100).map(|f| frame(cache, f).abs()).fold(0.0, f32::max)
}

#[test]
fn lists_plugin_parameters() {
    let host = fixture_host();
    let params = host
        .resolve(SINE_ID, &PluginSettings::default())
        .expect("resolve")
        .params()
        .expect("params");
    assert_eq!(params.len(), 1);
    let level = &params[0];
    assert_eq!(level.id, SINE_LEVEL_PARAM);
    assert_eq!((level.name.as_str(), level.module.as_str()), ("Level", "Output"));
    assert_eq!((level.min, level.max, level.default), (0.0, 1.0, 0.25));
    assert!(level.flags.automatable && !level.flags.readonly);
    let gain = host
        .resolve(GAIN_ID, &PluginSettings::default())
        .expect("resolve")
        .params()
        .expect("params");
    assert!(gain.is_empty(), "a plugin without the params extension lists none");
}

#[test]
fn embedded_parameter_values_apply_from_the_first_frame() {
    let host = fixture_host();
    let default = note_peak(&sine_cache(&host));
    let half = note_peak(&sine_cache_with(&host, &level(0.125)));
    assert!((default - 0.25).abs() < 0.01, "default level: {default}");
    assert!((half - 0.125).abs() < 0.01, "level 0.125: {half}");
    // Out-of-range values clamp to the parameter's range.
    let clamped = note_peak(&sine_cache_with(&host, &level(5.0)));
    assert!((clamped - 1.0).abs() < 0.02, "clamped to 1.0: {clamped}");
}

#[test]
fn unknown_parameter_is_rejected() {
    let host = fixture_host();
    let settings = PluginSettings {
        state: None,
        params: vec![ParamSetting { id: 99, value: 1.0 }],
    };
    let error = host
        .resolve(SINE_ID, &settings)
        .expect("resolve")
        .create(config(RenderMode::Offline))
        .err()
        .expect("error");
    assert_eq!(
        error,
        PluginHostError::UnknownParam {
            id: SINE_ID.into(),
            param: 99
        }
    );
}

#[test]
fn editors_report_gui_edits_as_saved_state() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let host = fixture_host();

    // Opening and closing without an edit reports no state.
    let (events, received) = channel();
    let editor = host
        .open_editor(GAIN_ID, &settings(Some(&gain_state(0.25))), "Gain", move |event| {
            let _ = events.send(event);
        })
        .expect("open editor");
    assert!(editor.is_open());
    editor.close();
    assert!(!editor.is_open());
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)),
        Ok(EditorEvent::Closed)
    );

    // A GUI edit is saved after the save delay and reported as the plugin's state.
    aura_clap_fixture::simulate_gui_edit(0.75);
    let (events, received) = channel();
    let editor = host
        .open_editor(GAIN_ID, &PluginSettings::default(), "Gain", move |event| {
            let _ = events.send(event);
        })
        .expect("open editor");
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)),
        Ok(EditorEvent::State(gain_state(0.75)))
    );
    editor.close();
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)),
        Ok(EditorEvent::Closed)
    );
}

#[test]
fn a_plugin_without_a_gui_has_no_editor() {
    let host = fixture_host();
    let error = host
        .open_editor(SINE_ID, &PluginSettings::default(), "Sine", |_| {})
        .expect_err("no GUI");
    assert_eq!(error, PluginHostError::NoGui { id: SINE_ID.into() });
}
