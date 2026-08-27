# Shared Deep Sea benchmark fixture, sourced by the presentation benchmark
# wrappers. Both arms must measure the same workload, so the fixture they build
# lives in one place rather than being kept in step by hand.
#
# Callers set SCRIPT_DIR and REPO_ROOT, then call `build_deep_sea_fixture`,
# which sets FIXTURE and installs the cleanup trap.

build_deep_sea_fixture() {
  local scratch_root=${TMPDIR:-/tmp}
  FIXTURE=$(mktemp -d "$scratch_root/clonk-rust-deep-sea-benchmark.XXXXXX")
  # shellcheck disable=SC2317  # invoked through the trap below.
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
  local duplicate_sheet
  for duplicate_sheet in GUICaption.png GUIScroll.png GUIProgress.png; do
    rm -f "$FIXTURE/Hazard.c4f/Graphics.c4g/$duplicate_sheet"
  done
  local scenario_text="$FIXTURE/Hazard.c4f/CTF_DeepSea.c4s/Scenario.txt"
  awk '!/^Origin=/' "$scenario_text" > "$scenario_text.tmp"
  mv "$scenario_text.tmp" "$scenario_text"
  cp "$SCRIPT_DIR/deep-sea-gpu-benchmark.ini" "$FIXTURE/config.ini"
  cp "$REPO_ROOT/crates/clonk-engine/tests/fixtures/embedded_player.c4p" \
    "$FIXTURE/Profiler-A.c4p"
  cp "$REPO_ROOT/crates/clonk-engine/tests/fixtures/embedded_player.c4p" \
    "$FIXTURE/Profiler-B.c4p"
}

# Every wrapper takes the same optional measurement-seconds argument.
require_positive_measurement_seconds() {
  case "$1" in
    ''|*[!0-9]*|0)
      echo "usage: $2 [positive-measurement-seconds]" >&2
      exit 64
      ;;
  esac
}

require_release_binary() {
  if [[ ! -x "$1" ]]; then
    echo "release binary not found: $1" >&2
    echo "build it with: cargo build --release --offline --locked -p clonk-app --bin clonk-app" >&2
    exit 66
  fi
}
