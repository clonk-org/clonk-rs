#!/usr/bin/env bash

set -euo pipefail

wrapper_path=$(realpath "$0")

find_real_cargo() {
    local path_entry candidate candidate_path
    local -a path_entries
    IFS=: read -r -a path_entries <<< "${PATH:-}"
    for path_entry in "${path_entries[@]}"; do
        [[ -n "$path_entry" ]] || path_entry=.
        candidate="$path_entry/cargo"
        [[ -x "$candidate" ]] || continue
        candidate_path=$(realpath "$candidate" 2>/dev/null) || continue
        [[ "$candidate_path" == "$wrapper_path" ]] && continue
        printf '%s\n' "$candidate"
        return 0
    done
    return 1
}

# Cargo can sanitize the CI-only LC_* variables before launching a nested
# command while still rewriting CARGO to this wrapper. Resolve past the
# wrapper in that environment instead of making presentation tooling fail.
real_cargo=${LC_REAL_CARGO:-${CARGO:-}}
resolved_cargo=
if [[ -n "$real_cargo" ]]; then
    resolved_cargo=$(realpath "$real_cargo" 2>/dev/null) || true
fi
if [[ -z "$real_cargo" || -z "$resolved_cargo" || "$resolved_cargo" == "$wrapper_path" ]]; then
    real_cargo=$(find_real_cargo) || {
        echo "LC_REAL_CARGO must name the real Cargo executable" >&2
        exit 127
    }
fi
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
