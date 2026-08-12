#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
MEASUREMENT_SECONDS=${1:-20}
BINARY=${LC_APP_BINARY:-$REPO_ROOT/target/release/clonk-app}

case "$MEASUREMENT_SECONDS" in
  ''|*[!0-9]*|0)
    echo "usage: $0 [positive-measurement-seconds]" >&2
    exit 64
    ;;
esac
if [[ ! -x "$BINARY" ]]; then
  echo "release binary not found: $BINARY" >&2
  echo "build it with: cargo build --release --offline --locked -p clonk-app --bin clonk-app" >&2
  exit 66
fi

SCRATCH_ROOT=${TMPDIR:-/tmp}
FIXTURE=$(mktemp -d "$SCRATCH_ROOT/clonk-rust-deep-sea-gpu-benchmark.XXXXXX")
cleanup() {
  find "$FIXTURE" -depth -delete
}
trap cleanup EXIT HUP INT TERM

# Hazard inherits three process-global GUI sheets whose duplicate binding is
# deliberately rejected by the Rust parity guard. They are unrelated to the
# running viewport; copy the group without them and remove Origin so lookup
# stays inside this self-contained benchmark fixture. Use tools available by
# default on both macOS and Linux.
mkdir -p "$FIXTURE/Hazard.c4f"
cp -R "$REPO_ROOT/content/Hazard.c4f/." "$FIXTURE/Hazard.c4f/"
for duplicate_sheet in GUICaption.png GUIScroll.png GUIProgress.png; do
  rm -f "$FIXTURE/Hazard.c4f/Graphics.c4g/$duplicate_sheet"
done
scenario_text="$FIXTURE/Hazard.c4f/CTF_DeepSea.c4s/Scenario.txt"
awk '!/^Origin=/' "$scenario_text" > "$scenario_text.tmp"
mv "$scenario_text.tmp" "$scenario_text"
cp "$SCRIPT_DIR/deep-sea-gpu-benchmark.ini" "$FIXTURE/config.ini"
cp "$REPO_ROOT/crates/clonk-engine/tests/fixtures/embedded_player.c4p" \
  "$FIXTURE/Profiler-A.c4p"
cp "$REPO_ROOT/crates/clonk-engine/tests/fixtures/embedded_player.c4p" \
  "$FIXTURE/Profiler-B.c4p"

benchmark_log="$FIXTURE/benchmark.log"
set +e
LC_PIN_SEED=1 \
LC_APP_PRESENTATION_BENCHMARK_SECONDS=$MEASUREMENT_SECONDS \
LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK=1 \
LC_APP_PRESENTATION_BENCHMARK_PLAYER_TEAMS=1,2 \
  "$BINARY" \
  --config "$FIXTURE/config.ini" \
  "$FIXTURE/Hazard.c4f/CTF_DeepSea.c4s" \
  "$FIXTURE/Profiler-A.c4p" \
  "$FIXTURE/Profiler-B.c4p" \
  2>&1 | tee "$benchmark_log"
binary_status=${PIPESTATUS[0]}
set -e
if (( binary_status != 0 )); then
  exit "$binary_status"
fi

context_count=$(awk '$1 == "LC_APP_PRESENTATION_BENCHMARK_CONTEXT" { count++ } END { print count + 0 }' "$benchmark_log")
if [[ "$context_count" != 1 ]]; then
  echo "expected exactly one LC_APP_PRESENTATION_BENCHMARK_CONTEXT line; found $context_count" >&2
  exit 1
fi
context_line=$(awk '$1 == "LC_APP_PRESENTATION_BENCHMARK_CONTEXT" { print }' "$benchmark_log")
expected_context=(
  "runtime_players=2"
  "synchronized_player_infos=2"
  "activated_nonhost_clients=0"
  "runtime_crew_objects=2"
  "runtime_players_with_live_crew=2"
  "runtime_players_with_exactly_one_live_sf5b_crew=0"
  "runtime_st5b_objects_at_measurement_start=0"
  "runtime_st5b_objects_at_measurement_end=0"
)
for expected in "${expected_context[@]}"; do
  case " $context_line " in
    *" $expected "*) ;;
    *)
      echo "expected $expected in Deep Sea benchmark context" >&2
      exit 1
      ;;
  esac
done
