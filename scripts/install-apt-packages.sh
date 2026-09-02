#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
    echo "usage: ${0##*/} PACKAGE..." >&2
    exit 2
fi

# Every call site wraps this script in a 10-minute step. The ladder below must
# therefore finish well inside that: a step killed mid-attempt reports only
# "the action has timed out", which names neither the package nor the mirror,
# while an exhausted ladder reports both. `test_ci_latency.py` pins this budget
# against the smallest `timeout-minutes` that runs the script.
readonly LADDER_BUDGET_SECONDS=480
readonly MIRRORS=/etc/apt/apt-mirrors.txt

# apt has no default per-connection timeout, so a mirror that completes the
# handshake and then stops sending spends the whole wall-clock bound below
# transferring nothing -- two such stalls consumed eight of ten minutes on
# 2026-09-01. `Acquire::*::Timeout` is an inactivity timeout, which turns a
# dead socket into a fast error the ladder can act on.
readonly APT_OPTIONS=(
    -o Acquire::http::Timeout=15
    -o Acquire::https::Timeout=15
    -o Acquire::Retries=2
)

# Clamp every apt invocation to whatever is left of the ladder budget so the
# ladder cannot overrun it however the nominal bounds are tuned.
bounded_apt() {
    local nominal=$1
    shift
    local budget=$((deadline - SECONDS))
    ((budget > 0)) || return 1
    if ((nominal < budget)); then
        budget=$nominal
    fi
    timeout "$budget" sudo apt-get "${APT_OPTIONS[@]}" "$@"
}

apt_install() { bounded_apt 120 install --yes --no-install-recommends "$@"; }
apt_refresh() { bounded_apt 90 update; }

# Each apt-get invocation re-reads the runner's mirror list and picks the same
# head entry, so without this every retry below repeats the request that just
# failed: 27 of 27 package requests in the 2026-09-01 stall went to the one
# host that was stalling. Demote the entry that just failed and keep the
# canonical archive present so there is always somewhere else to go.
rotate_mirror() {
    [[ -r $MIRRORS ]] || return 0
    local first rest
    first=$(head -n 1 "$MIRRORS")
    rest=$(tail -n +2 "$MIRRORS")
    printf '%s\n' "$rest" "http://archive.ubuntu.com/ubuntu/" "$first" |
        awk 'NF && !seen[$0]++' |
        sudo tee "$MIRRORS" >/dev/null
    echo "apt mirror rotated; head is now $(head -n 1 "$MIRRORS")" >&2
}

deadline=$((SECONDS + LADDER_BUDGET_SECONDS))

for attempt in 1 2 3; do
    ((SECONDS < deadline)) || break
    if apt_install "$@"; then
        exit 0
    fi
    rotate_mirror
    apt_refresh || true
    if apt_install "$@"; then
        exit 0
    fi
    rotate_mirror
    ((SECONDS < deadline)) || break
    echo "apt attempt ${attempt} failed; retrying" >&2
    sleep $((attempt * 5))
done

echo "::error::apt install did not complete within ${LADDER_BUDGET_SECONDS}s" >&2
exit 1
