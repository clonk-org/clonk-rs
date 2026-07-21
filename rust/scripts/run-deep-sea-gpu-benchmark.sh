#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUST_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
REPO_ROOT=$(cd "$RUST_ROOT/.." && pwd)
MEASUREMENT_SECONDS=${1:-20}
BINARY=${LC_APP_BINARY:-$RUST_ROOT/target/release/lc-app}

case "$MEASUREMENT_SECONDS" in
  ''|*[!0-9]*|0)
    echo "usage: $0 [positive-measurement-seconds]" >&2
    exit 64
    ;;
esac
if [[ ! -x "$BINARY" ]]; then
  echo "release binary not found: $BINARY" >&2
  echo "build it with: cargo build --release --offline --locked -p lc-app --bin lc-app" >&2
  exit 66
fi

FIXTURE=$(mktemp -d /private/tmp/lc-deep-sea-gpu-benchmark.XXXXXX)
cleanup() {
  find "$FIXTURE" -depth -delete
}
trap cleanup EXIT HUP INT TERM

# Hazard inherits three process-global GUI sheets whose duplicate binding is
# deliberately rejected by the Rust parity guard. They are unrelated to the
# running viewport; copy the group without them and remove Origin so lookup
# stays inside this self-contained benchmark fixture.
rsync -a \
  --exclude='Graphics.c4g/GUICaption.png' \
  --exclude='Graphics.c4g/GUIScroll.png' \
  --exclude='Graphics.c4g/GUIProgress.png' \
  "$REPO_ROOT/content/Hazard.c4f/" "$FIXTURE/Hazard.c4f/"
perl -ni -e 'print unless /^Origin=/' \
  "$FIXTURE/Hazard.c4f/CTF_DeepSea.c4s/Scenario.txt"
cp "$SCRIPT_DIR/deep-sea-gpu-benchmark.ini" "$FIXTURE/config.ini"

LC_PIN_SEED=1 \
LC_APP_PRESENTATION_BENCHMARK_SECONDS=$MEASUREMENT_SECONDS \
LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK=1 \
  "$BINARY" \
  --config "$FIXTURE/config.ini" \
  "$FIXTURE/Hazard.c4f/CTF_DeepSea.c4s" \
  "$RUST_ROOT/crates/lc-engine/tests/fixtures/embedded_player.c4p"
