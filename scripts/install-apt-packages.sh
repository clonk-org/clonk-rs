#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
    echo "usage: ${0##*/} PACKAGE..." >&2
    exit 2
fi

# Try the cached package index first; refresh only after a bounded failure so a
# stalled mirror or dpkg lock cannot consume the whole CI job.
apt_install() {
    timeout 240 sudo apt-get install --yes --no-install-recommends "$@"
}
apt_refresh() { timeout 180 sudo apt-get update; }

for attempt in 1 2 3; do
    if apt_install "$@"; then
        exit 0
    fi
    apt_refresh || true
    if apt_install "$@"; then
        exit 0
    fi
    echo "apt attempt ${attempt} failed; retrying" >&2
    sleep $((attempt * 15))
done
exit 1
