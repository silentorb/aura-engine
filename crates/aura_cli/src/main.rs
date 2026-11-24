//! Aura command-line interface — graph-driven audio rendering and Miro-profile sync helpers.

use aura_integration::{
    sync_content_root, parse_duration_spec, render_graph_file_to_pcm, write_pcm_to_wav,
    DurationSpec, HostError, IntegrationError, SampleSpec,
};
use aura_sample::{SampleRate, SampleRateError};
use std::env;
use std::path::PathBuf;
use thiserror::Error;

const DEFAULT_DURATION: &str = "10s";
const DEFAULT_TEMPO: f64 = 120.0;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    SampleRate(#[from] SampleRateError),
    #[error("{0}")]
    Duration(#[from] aura_integration::DurationParseError),
    #[error("{0}")]
    Integration(#[from] IntegrationError),
    #[error("{0}")]
    Host(#[from] HostError),
    #[error("missing required --graph path")]
    MissingGraph,
    #[error("missing required --output path")]
    MissingOutput,
    #[error("missing required --sync-root path")]
    MissingSyncRoot,
    #[error("missing required --sync-db path")]
    MissingSyncDb,
    #[error("invalid --{flag} value `{value}`: {reason}")]
    InvalidArg {
        flag: &'static str,
        value: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderGraphConfig {
    pub graph: PathBuf,
    /// Tome project config whose records supply nodes the graph references (e.g. notations).
    pub project: Option<PathBuf>,
    pub output: PathBuf,
    pub duration: DurationSpec,
    pub sample_rate: SampleRate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncConfig {
    pub root: PathBuf,
    pub db: PathBuf,
    pub pull: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CliCommand {
    Render(RenderGraphConfig),
    Sync(SyncConfig),
}

fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<CliCommand, CliError> {
    let mut graph = None;
    let mut project = None;
    let mut output = None;
    let mut duration = DEFAULT_DURATION.to_string();
    let mut tempo = DEFAULT_TEMPO;
    let mut sample_rate = SampleRate::default();
    let mut sync_root = None;
    let mut sync_db = None;
    let mut sync_pull = None;
    let mut sync_mode = false;

    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_ref() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "sync" => {
                sync_mode = true;
            }
            "--sync-root" => {
                sync_mode = true;
                sync_root = Some(next_value(&mut iter, "sync-root")?);
            }
            "--sync-db" => {
                sync_mode = true;
                sync_db = Some(next_value(&mut iter, "sync-db")?);
            }
            "--sync-pull" => {
                sync_mode = true;
                sync_pull = Some(next_value(&mut iter, "sync-pull")?);
            }
            "--graph" | "-g" => {
                graph = Some(next_value(&mut iter, "graph")?);
            }
            "--project" | "-p" => {
                project = Some(next_value(&mut iter, "project")?);
            }
            "--output" | "-o" => {
                output = Some(next_value(&mut iter, "output")?);
            }
            "--duration" | "-d" => {
                duration = next_value(&mut iter, "duration")?;
            }
            "--tempo" => {
                let value = next_value(&mut iter, "tempo")?;
                tempo = value.parse().map_err(|_| CliError::InvalidArg {
                    flag: "tempo",
                    value,
                    reason: "expected a positive number".into(),
                })?;
            }
            "--sample-rate" | "-r" => {
                let value = next_value(&mut iter, "sample-rate")?;
                let hz: u32 = value.parse().map_err(|_| CliError::InvalidArg {
                    flag: "sample-rate",
                    value: value.clone(),
                    reason: "expected an integer sample rate".into(),
                })?;
                sample_rate = SampleRate::new(hz)?;
            }
            other => {
                return Err(CliError::InvalidArg {
                    flag: "argument",
                    value: other.to_string(),
                    reason: "unknown flag".into(),
                });
            }
        }
    }

    if sync_mode {
        let root = sync_root.ok_or(CliError::MissingSyncRoot)?;
        let db = sync_db.ok_or(CliError::MissingSyncDb)?;
        return Ok(CliCommand::Sync(SyncConfig {
            root: PathBuf::from(root),
            db: PathBuf::from(db),
            pull: sync_pull,
        }));
    }

    let graph = graph.ok_or(CliError::MissingGraph)?;
    let output = output.ok_or(CliError::MissingOutput)?;
    let duration = parse_duration_spec(&duration, tempo)?;

    Ok(CliCommand::Render(RenderGraphConfig {
        graph: PathBuf::from(graph),
        project: project.map(PathBuf::from),
        output: PathBuf::from(output),
        duration,
        sample_rate,
    }))
}

fn next_value(
    iter: &mut std::iter::Peekable<impl Iterator<Item = impl AsRef<str>>>,
    flag: &'static str,
) -> Result<String, CliError> {
    iter.next()
        .map(|value| value.as_ref().to_string())
        .ok_or_else(|| CliError::InvalidArg {
            flag,
            value: String::new(),
            reason: "missing value".into(),
        })
}

fn print_help() {
    eprintln!(
        "aura — render an Imp graph to WAV, or sync a content root (Miro profile)\n\n\
         Usage:\n  \
         aura --graph PATH --output PATH [OPTIONS]\n  \
         aura sync --sync-root PATH --sync-db PATH [--sync-pull REL]\n\n\
         Render options:\n  \
         -g, --graph PATH         Imp graph JSON file\n  \
         -p, --project PATH       Tome project config resolving nodes the graph references\n  \
         -o, --output PATH        Output WAV path\n  \
         -d, --duration SPEC      Duration: seconds (10, 10s) or measures (2m)\n  \
         --tempo BPM              Tempo for measure durations (default: 120)\n  \
         -r, --sample-rate HZ     Sample rate (default: 44100)\n\n\
         Sync options (Mir fabric via imp_mir):\n  \
         --sync-root PATH         Text content root (Aura JSON files)\n  \
         --sync-db PATH           SQLite projection path\n  \
         --sync-pull REL          Optional relative path to pull/verify after sync\n  \
         -h, --help               Show this help"
    );
}

pub fn render_graph(config: &RenderGraphConfig) -> Result<(), CliError> {
    let spec = SampleSpec {
        sample_rate: config.sample_rate,
        duration: config.duration,
    };
    let pcm = render_graph_file_to_pcm(&config.graph, spec, config.project.as_deref())?;
    write_pcm_to_wav(&config.output, &pcm, config.sample_rate)?;
    Ok(())
}

fn run_sync(config: &SyncConfig) -> Result<(), CliError> {
    let graph = sync_content_root(&config.root, &config.db, config.pull.as_deref())?;
    eprintln!(
        "Sync: {} → {}",
        config.root.display(),
        config.db.display()
    );
    if let Some(id) = &config.pull {
        let graph = graph.expect("pull requested");
        eprintln!(
            "Pulled `{id}` ({} nodes, {} edges)",
            graph.nodes.len(),
            graph.edges.len()
        );
    }
    Ok(())
}

fn main() {
    let command = match parse_args(env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error}");
            eprintln!("Run aura --help for usage.");
            std::process::exit(1);
        }
    };

    match command {
        CliCommand::Render(config) => {
            if let Err(error) = render_graph(&config) {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
            eprintln!(
                "Wrote {} from {}",
                config.output.display(),
                config.graph.display()
            );
        }
        CliCommand::Sync(config) => {
            if let Err(error) = run_sync(&config) {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_requires_graph_and_output() {
        let err = parse_args(["--graph", "demos/sine.json"]).unwrap_err();
        assert!(matches!(err, CliError::MissingOutput));
    }

    #[test]
    fn parse_args_reads_duration_and_sample_rate() {
        let config = parse_args([
            "--graph",
            "demos/sine.json",
            "--output",
            "output/sine.wav",
            "--duration",
            "1s",
            "--sample-rate",
            "44100",
        ])
        .expect("parse");

        match config {
            CliCommand::Render(config) => {
                assert_eq!(config.duration, DurationSpec::Seconds(1.0));
                assert_eq!(config.sample_rate, SampleRate::RATE_44100);
            }
            CliCommand::Sync(_) => panic!("expected render"),
        }
    }

    #[test]
    fn parse_sync_args() {
        let command = parse_args([
            "sync",
            "--sync-root",
            "demos",
            "--sync-db",
            "/tmp/aura.projection.db",
            "--sync-pull",
            "demo-01.json",
        ])
        .expect("parse");
        match command {
            CliCommand::Sync(cfg) => {
                assert_eq!(cfg.root, PathBuf::from("demos"));
                assert_eq!(cfg.pull.as_deref(), Some("demo-01.json"));
            }
            CliCommand::Render(_) => panic!("expected sync"),
        }
    }
}
