#!/usr/bin/env bash

set -euo pipefail

source_path=${LC_NEXTEST_JUNIT_SOURCE:-target/nextest/default/junit.xml}
destination_dir=${LC_NEXTEST_JUNIT_DIR:?LC_NEXTEST_JUNIT_DIR must name the retained-report directory}

if [[ ! -f "$source_path" ]]; then
    exit 0
fi

mkdir -p "$destination_dir"
checksum=$(sha256sum "$source_path")
checksum=${checksum%% *}
sequence=0
while :; do
    destination="$destination_dir/junit-${sequence}-${checksum}.xml"
    if [[ ! -e "$destination" ]]; then
        cp "$source_path" "$destination"
        break
    fi
    ((sequence += 1))
done
