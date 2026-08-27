#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
MEASUREMENT_SECONDS=${1:-20}
BINARY=${LC_APP_BINARY:-$REPO_ROOT/target/release/clonk-app}

# shellcheck source=scripts/deep-sea-benchmark-fixture.sh
. "$SCRIPT_DIR/deep-sea-benchmark-fixture.sh"

require_positive_measurement_seconds "$MEASUREMENT_SECONDS" "$0"
require_release_binary "$BINARY"
build_deep_sea_fixture

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
