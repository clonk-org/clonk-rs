#!/usr/bin/env bash
set -euo pipefail

# The software-presentation arm of the Deep Sea benchmark: the same workload
# and fixture as the retained-GPU wrapper beside it, presented through
# SoftwarePresenter instead.
#
# It deliberately does not assert the native tick budget. Software presentation
# is measured here, not qualified: the point is to report what the fallback
# costs, and a machine that cannot hold 28ms through it is evidence rather than
# a failure.

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
LC_SOFTWARE_PRESENTATION=1 \
LC_APP_PRESENTATION_BENCHMARK_SECONDS=$MEASUREMENT_SECONDS \
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

metric_count=$(awk '$1 == "LC_APP_PRESENTATION_BENCHMARK" && $2 ~ /^elapsed_seconds=/ { count++ } END { print count + 0 }' "$benchmark_log")
if [[ "$metric_count" != 1 ]]; then
  echo "expected exactly one LC_APP_PRESENTATION_BENCHMARK metric line; found $metric_count" >&2
  exit 1
fi
metric_line=$(awk '$1 == "LC_APP_PRESENTATION_BENCHMARK" && $2 ~ /^elapsed_seconds=/ { print }' "$benchmark_log")

field() {
  awk -v key="$1" '{
    for (i = 2; i <= NF; i++) {
      split($i, pair, "=")
      if (pair[1] == key) { print pair[2]; exit }
    }
  }' <<< "$metric_line"
}

# The whole point of this arm is the software destination, and a run that
# quietly built a GPU adapter measures the other one while looking identical.
# Fail loudly rather than publish a mislabelled distribution.
cpu_submissions=$(field cpu_present_submissions)
gpu_submissions=$(field retained_gpu_present_submissions)
if [[ -z "$cpu_submissions" || -z "$gpu_submissions" ]]; then
  echo "benchmark line carries no presentation submission counts" >&2
  exit 1
fi
if (( gpu_submissions != 0 )); then
  echo "the software arm presented $gpu_submissions retained-GPU frames; LC_SOFTWARE_PRESENTATION did not take" >&2
  exit 1
fi
if (( cpu_submissions == 0 )); then
  echo "no software presentations were measured (cpu_present_submissions=0)" >&2
  exit 1
fi
