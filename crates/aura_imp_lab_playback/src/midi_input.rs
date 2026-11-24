//! MIDI input for the Lab: the configured device (by port name) and channel filter, kept
//! connected across unplug and replug, monitored through streaming voices, and fed to a
//! recording take.
//!
//! Lab stores the configuration as two extension settings (`aura.midi.input`,
//! `aura.midi.channels`) and delivers them in a `settings` event. The extension publishes the
//! port list (`aura.midi.ports`) and connection status (`aura.midi.status`) as topics when they
//! change.

use crate::audition::{voice_off, voice_on, MIDI_VOICE_ID_BASE};
use crate::{publish, recording};
use aura_io_midi::{ChannelFilter, MidiInput, MidiInputEvent, PortLister, SystemPorts};
use aura_sequencing::capture::CapturedKind;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const SETTING_INPUT: &str = "aura.midi.input";
pub const SETTING_CHANNELS: &str = "aura.midi.channels";
pub const TOPIC_PORTS: &str = "aura.midi.ports";
pub const TOPIC_STATUS: &str = "aura.midi.status";

/// How often the background thread wakes (status flush) and rescans ports (every
/// [`RESCAN_EVERY`] wakes). midir has no hot-plug notification, so ports are polled here,
/// privately; Lab only sees published changes.
const TICK: Duration = Duration::from_millis(100);
const RESCAN_EVERY: u32 = 15;
/// Least time between activity-driven status publishes.
const STATUS_THROTTLE: Duration = Duration::from_millis(100);

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MidiInputConfig {
    pub device: Option<String>,
    pub channels: ChannelFilter,
}

impl MidiInputConfig {
    /// Reads the configuration from setting values (unknown keys ignored; absent keys keep
    /// `current`).
    pub fn from_settings(values: &Map<String, Value>, current: &Self) -> Self {
        let device = match values.get(SETTING_INPUT) {
            Some(Value::String(name)) if !name.is_empty() => Some(name.clone()),
            Some(_) => None,
            None => current.device.clone(),
        };
        let channels = match values.get(SETTING_CHANNELS) {
            Some(Value::Array(items)) => {
                let channels: Vec<u8> = items
                    .iter()
                    .filter_map(|v| match v {
                        Value::String(s) => s.parse().ok(),
                        Value::Number(n) => n.as_u64().and_then(|n| u8::try_from(n).ok()),
                        _ => None,
                    })
                    .collect();
                ChannelFilter::from_channels(&channels)
            }
            Some(_) => ChannelFilter::ALL,
            None => current.channels,
        };
        Self { device, channels }
    }
}

/// Connection lifecycle of the configured device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Off,
    Connected(String),
    /// Configured but not present (unplugged, or held exclusively by another app).
    Waiting(String),
    Error(String, String),
}

/// What the manager should do given the configuration, connection and present ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Open(String),
    /// Close the connection (device unplugged, or deconfigured).
    Close,
}

/// Decides the next step. `connected` is the name of the open port, if any.
pub fn next_action(desired: Option<&str>, connected: Option<&str>, ports: &[String]) -> Action {
    match (desired, connected) {
        (None, None) => Action::Nothing,
        (None, Some(_)) => Action::Close,
        (Some(name), Some(open)) if open != name => Action::Close,
        (Some(name), Some(_)) => {
            if ports.iter().any(|p| p == name) {
                Action::Nothing
            } else {
                Action::Close
            }
        }
        (Some(name), None) => {
            if ports.iter().any(|p| p == name) {
                Action::Open(name.to_string())
            } else {
                Action::Nothing
            }
        }
    }
}

/// The status item value for a state and the last note seen.
pub fn status_value(state: &ConnectionState, last: Option<&LastNote>) -> Value {
    match state {
        ConnectionState::Off => json!({"text": "MIDI: off", "level": "off"}),
        ConnectionState::Connected(name) => {
            let text = match last {
                Some(note) => format!(
                    "MIDI: {name} · {} vel {}",
                    note_label(note.semitone),
                    (note.velocity * 127.0).round() as u8
                ),
                None => format!("MIDI: {name}"),
            };
            json!({"text": text, "level": "ok"})
        }
        ConnectionState::Waiting(name) => {
            json!({"text": format!("MIDI: {name} (disconnected)"), "level": "warn"})
        }
        ConnectionState::Error(name, message) => {
            json!({"text": format!("MIDI: {name}: {message}"), "level": "error"})
        }
    }
}

/// Choice options for the input setting: "None" then the present ports.
pub fn port_options(ports: &[String]) -> Value {
    let mut options = vec![json!({"value": "", "label": "None"})];
    options.extend(ports.iter().map(|p| json!({"value": p, "label": p})));
    Value::Array(options)
}

const PITCH_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Scientific pitch label (60 → `C4`).
pub fn note_label(semitone: i16) -> String {
    let pitch = PITCH_NAMES[semitone.rem_euclid(12) as usize];
    format!("{pitch}{}", semitone.div_euclid(12) - 1)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LastNote {
    pub semitone: i16,
    pub velocity: f32,
}

/// Monitoring voices for the device, honoring the sustain pedal.
#[derive(Debug, Default)]
struct Monitor {
    pedal: bool,
    /// Keys down now.
    down: BTreeSet<i16>,
    /// Keys released while the pedal is down (still sounding).
    sustained: BTreeSet<i16>,
}

fn voice_id(semitone: i16) -> u64 {
    MIDI_VOICE_ID_BASE + u64::from(semitone as u16)
}

impl Monitor {
    fn apply(&mut self, kind: CapturedKind) {
        let result = match kind {
            CapturedKind::KeyDown { semitone, velocity } => {
                self.down.insert(semitone.0);
                self.sustained.remove(&semitone.0);
                voice_on(voice_id(semitone.0), semitone.0, velocity)
            }
            CapturedKind::KeyUp { semitone } => {
                if !self.down.remove(&semitone.0) {
                    Ok(())
                } else if self.pedal {
                    self.sustained.insert(semitone.0);
                    Ok(())
                } else {
                    voice_off(voice_id(semitone.0))
                }
            }
            CapturedKind::SustainDown => {
                self.pedal = true;
                Ok(())
            }
            CapturedKind::SustainUp => {
                self.pedal = false;
                let released: Vec<i16> = std::mem::take(&mut self.sustained).into_iter().collect();
                released
                    .into_iter()
                    .try_for_each(|semitone| voice_off(voice_id(semitone)))
            }
        };
        // Monitoring is best effort (no audio device): recording still captures.
        let _ = result;
    }

    /// Releases every sounding voice (device gone); returns the keys that were down.
    fn release_all(&mut self) -> Vec<i16> {
        let down: Vec<i16> = self.down.iter().copied().collect();
        for semitone in self.down.iter().chain(self.sustained.iter()) {
            let _ = voice_off(voice_id(*semitone));
        }
        self.down.clear();
        self.sustained.clear();
        self.pedal = false;
        down
    }
}

struct Manager {
    config: MidiInputConfig,
    state: ConnectionState,
    connection: Option<MidiInput>,
    ports: Vec<String>,
    monitor: Monitor,
    last: Option<LastNote>,
    status_dirty: bool,
    last_status_publish: Option<Instant>,
}

fn manager() -> &'static Mutex<Manager> {
    static MANAGER: OnceLock<Mutex<Manager>> = OnceLock::new();
    MANAGER.get_or_init(|| {
        Mutex::new(Manager {
            config: MidiInputConfig::default(),
            state: ConnectionState::Off,
            connection: None,
            ports: Vec::new(),
            monitor: Monitor::default(),
            last: None,
            status_dirty: true,
            last_status_publish: None,
        })
    })
}

impl Manager {
    fn publish_status(&mut self) {
        publish(TOPIC_STATUS, &status_value(&self.state, self.last.as_ref()));
        self.status_dirty = false;
        self.last_status_publish = Some(Instant::now());
    }

    /// Applies `action`; returns a connection to drop once the lock is released (dropping a
    /// midir connection joins its thread, which may be waiting on this lock).
    fn apply(&mut self, action: Action) -> Option<MidiInput> {
        match action {
            Action::Nothing => {
                let expected = match &self.config.device {
                    None => ConnectionState::Off,
                    Some(name) if self.connection.is_some() => {
                        ConnectionState::Connected(name.clone())
                    }
                    Some(name) => match &self.state {
                        ConnectionState::Error(n, _) if n == name => self.state.clone(),
                        _ => ConnectionState::Waiting(name.clone()),
                    },
                };
                if expected != self.state {
                    self.state = expected;
                    self.status_dirty = true;
                }
                None
            }
            Action::Close => {
                let closed = self.connection.take();
                self.release_held();
                self.state = match &self.config.device {
                    None => ConnectionState::Off,
                    Some(name) => ConnectionState::Waiting(name.clone()),
                };
                self.last = None;
                self.status_dirty = true;
                closed
            }
            Action::Open(name) => {
                self.state = match MidiInput::open(&name, on_midi_event) {
                    Ok(connection) => {
                        self.connection = Some(connection);
                        ConnectionState::Connected(name)
                    }
                    Err(err) => ConnectionState::Error(name, err.to_string()),
                };
                self.status_dirty = true;
                None
            }
        }
    }

    fn release_held(&mut self) {
        let down = self.monitor.release_all();
        recording::release_keys(&down);
    }

    fn sync(&mut self) -> Option<MidiInput> {
        let action = next_action(
            self.config.device.as_deref(),
            self.connection.as_ref().map(MidiInput::name),
            &self.ports,
        );
        let closed = self.apply(action);
        // A close for a renamed device: open the new one right away.
        if closed.is_some() {
            let action = next_action(self.config.device.as_deref(), None, &self.ports);
            if let Action::Open(_) = action {
                self.apply(action);
            }
        }
        closed
    }
}

fn on_midi_event(event: MidiInputEvent) {
    let Ok(mut manager) = manager().lock() else {
        return;
    };
    if !manager.config.channels.allows(event.channel) {
        return;
    }
    manager.monitor.apply(event.kind);
    if let CapturedKind::KeyDown { semitone, velocity } = event.kind {
        manager.last = Some(LastNote {
            semitone: semitone.0,
            velocity,
        });
        let due = manager
            .last_status_publish
            .is_none_or(|at| at.elapsed() >= STATUS_THROTTLE);
        if due {
            manager.publish_status();
        } else {
            manager.status_dirty = true;
        }
    }
    recording::capture(event.timestamp_us, event.kind);
}

/// Applies a `settings` event's values.
pub fn apply_settings(values: &Map<String, Value>) {
    ensure_thread();
    let closed = {
        let Ok(mut manager) = manager().lock() else {
            return;
        };
        let config = MidiInputConfig::from_settings(values, &manager.config);
        if config == manager.config {
            return;
        }
        let renamed = config.device != manager.config.device;
        manager.config = config;
        if renamed && manager.ports.is_empty() {
            manager.ports = SystemPorts.names().unwrap_or_default();
        }
        let closed = manager.sync();
        manager.publish_status();
        closed
    };
    drop(closed);
}

fn rescan(lister: &dyn PortLister) {
    let ports = lister.names().unwrap_or_default();
    let closed = {
        let Ok(mut manager) = manager().lock() else {
            return;
        };
        if ports != manager.ports {
            publish(TOPIC_PORTS, &port_options(&ports));
            manager.ports = ports;
        }
        manager.sync()
    };
    drop(closed);
}

fn flush_status() {
    if let Ok(mut manager) = manager().lock() {
        if manager.status_dirty {
            manager.publish_status();
        }
    }
}

/// Starts the background rescan / status thread once. It publishes the port list and status
/// right away, then on change.
pub fn ensure_thread() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("aura-midi-input".into())
            .spawn(|| {
                publish(TOPIC_PORTS, &port_options(&[]));
                let mut tick = 0_u32;
                loop {
                    if tick.is_multiple_of(RESCAN_EVERY) {
                        rescan(&SystemPorts);
                    }
                    flush_status();
                    crate::audition::reclaim_voices();
                    tick = tick.wrapping_add(1);
                    std::thread::sleep(TICK);
                }
            });
    });
}

/// Name of the connected input, for tests and diagnostics.
pub fn connected_device() -> Option<String> {
    manager()
        .lock()
        .ok()?
        .connection
        .as_ref()
        .map(|c| c.name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn opens_when_the_configured_port_appears_and_closes_when_it_goes() {
        let none: [&str; 0] = [];
        assert_eq!(next_action(None, None, &ports(&["Keys"])), Action::Nothing);
        assert_eq!(
            next_action(Some("Keys"), None, &ports(&none)),
            Action::Nothing
        );
        assert_eq!(
            next_action(Some("Keys"), None, &ports(&["Pads", "Keys"])),
            Action::Open("Keys".into())
        );
        assert_eq!(
            next_action(Some("Keys"), Some("Keys"), &ports(&["Keys"])),
            Action::Nothing
        );
        assert_eq!(
            next_action(Some("Keys"), Some("Keys"), &ports(&["Pads"])),
            Action::Close,
            "unplugged"
        );
        assert_eq!(
            next_action(Some("Pads"), Some("Keys"), &ports(&["Pads", "Keys"])),
            Action::Close,
            "reconfigured"
        );
        assert_eq!(
            next_action(None, Some("Keys"), &ports(&["Keys"])),
            Action::Close
        );
    }

    #[test]
    fn config_from_settings() {
        let values: Map<String, Value> = serde_json::from_str(
            r#"{"aura.midi.input": "Keys", "aura.midi.channels": ["1", "10"], "other": 1}"#,
        )
        .unwrap();
        let config = MidiInputConfig::from_settings(&values, &MidiInputConfig::default());
        assert_eq!(config.device.as_deref(), Some("Keys"));
        assert!(config.channels.allows(10) && !config.channels.allows(2));

        let none: Map<String, Value> = serde_json::from_str(r#"{"aura.midi.input": ""}"#).unwrap();
        let cleared = MidiInputConfig::from_settings(&none, &config);
        assert_eq!(cleared.device, None);
        assert_eq!(
            cleared.channels, config.channels,
            "absent keys keep the current value"
        );
    }

    #[test]
    fn status_text_and_level() {
        assert_eq!(
            status_value(&ConnectionState::Off, None),
            json!({"text": "MIDI: off", "level": "off"})
        );
        let last = LastNote {
            semitone: 60,
            velocity: 96.0 / 127.0,
        };
        assert_eq!(
            status_value(&ConnectionState::Connected("Keys".into()), Some(&last)),
            json!({"text": "MIDI: Keys · C4 vel 96", "level": "ok"})
        );
        assert_eq!(
            status_value(&ConnectionState::Waiting("Keys".into()), None)["level"],
            "warn"
        );
        assert_eq!(
            status_value(&ConnectionState::Error("Keys".into(), "busy".into()), None)["level"],
            "error"
        );
    }

    #[test]
    fn port_options_start_with_none() {
        assert_eq!(
            port_options(&ports(&["Keys"])),
            json!([{"value": "", "label": "None"}, {"value": "Keys", "label": "Keys"}])
        );
        assert_eq!(note_label(69), "A4");
        assert_eq!(note_label(0), "C-1");
    }
}
