#!/usr/bin/env bash

set -euo pipefail

real_cargo=${LC_REAL_CARGO:?LC_REAL_CARGO must name the real Cargo executable}
helper=${LC_NEXTEST_JUNIT_HELPER:-scripts/retain-nextest-junit.sh}
source_path=${LC_NEXTEST_JUNIT_SOURCE:-${CARGO_TARGET_DIR:-target}/nextest/default/junit.xml}

# A native Cargo child (for example the one started by `cargo xtask parity
# verify`) resolves this executable through PATH or LC_CARGO_WRAPPER. Apply
# the same stale-report reset and snapshot around that invocation.
rm -f "$source_path"
status=0
"$real_cargo" "$@" || status=$?
retain=false
for argument in "$@"; do
    if [[ "$argument" == nextest ]]; then
        retain=true
        break
    fi
done
if [[ "$retain" == true ]]; then
    bash "$helper" || true
fi
exit "$status"
