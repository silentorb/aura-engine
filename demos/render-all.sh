#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build -p aura_cli --quiet

cargo run -p aura_cli --quiet -- \
  --graph demos/demo-01.json \
  --output output/demo-01.wav \
  --duration 8m \
  --tempo 120 \
  --sample-rate 44100

echo "Wrote output/demo-01.wav"

cargo run -p aura_cli --quiet -- \
  --graph demos/demo-02.json \
  --project lab-pack/demo-project/imp-project.json \
  --output output/demo-02.wav \
  --duration 8m \
  --tempo 120 \
  --sample-rate 44100

echo "Wrote output/demo-02.wav"

# Plugin demo: the arpeggio through Aura's fixture CLAP plugins (sine synth → gain), loaded as a
# real `.clap` bundle from the build directory.
cargo build -p aura_clap_fixture --quiet
plugin_dir="${CARGO_TARGET_DIR:-target}/clap-fixture"
mkdir -p "$plugin_dir"
cp "${CARGO_TARGET_DIR:-target}/debug/libaura_clap_fixture.so" "$plugin_dir/aura_clap_fixture.clap"

cargo run -p aura_cli --quiet -- \
  --graph demos/demo-03-plugin.json \
  --plugin-path "$plugin_dir" \
  --output output/demo-03-plugin.wav \
  --duration 4m \
  --tempo 120 \
  --sample-rate 44100

echo "Wrote output/demo-03-plugin.wav"

# Stereo demo: demo-01 with each part panned before the mix, so the generic `add` (and the
# instruments' generic nodes downstream of each pan) resolve to stereo.
cargo run -p aura_cli --quiet -- \
  --graph demos/demo-04-stereo.json \
  --output output/demo-04-stereo.wav \
  --duration 8m \
  --tempo 120 \
  --sample-rate 44100

echo "Wrote output/demo-04-stereo.wav"
