#!/usr/bin/env bash
set -euo pipefail

# Prepare a release.
#
# Works out the next version from Conventional Commit subjects, bumps the
# single workspace version, refreshes the lockfile and writes CHANGELOG.md.
#
# There is one version for the whole workspace: every crate inherits
# `version.workspace = true` and the workspace sets `publish = false`, so
# nothing reaches a registry and per-crate versions would carry no signal.
#
# This deliberately stops before committing, tagging or pushing: the release
# archives are built locally with `cargo xtask package` (packaging needs the
# private content submodule), and the gates should run against the bumped tree
# before any of it becomes permanent.
#
# Usage:
#   scripts/prepare-release.sh              # version inferred from commits
#   scripts/prepare-release.sh 1.2.3        # explicit version

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
tool_version=2.13.1

tool_root="${CLONK_GIT_CLIFF_ROOT:-$repo_root/target/release-tools/git-cliff-$tool_version}"
tool="$tool_root/bin/git-cliff"

if [[ ! -x "$tool" ]]; then
    cargo install \
        --locked \
        --version "$tool_version" \
        --root "$tool_root" \
        git-cliff
fi
if [[ $("$tool" --version) != "git-cliff $tool_version" ]]; then
    echo "unexpected git-cliff executable at $tool" >&2
    exit 1
fi

cd "$repo_root"

if ! git diff --quiet --no-ext-diff -- . ':!content'; then
    echo "working tree has uncommitted changes; commit or discard them first" >&2
    exit 1
fi

if [[ $# -gt 1 ]]; then
    echo "usage: $(basename "$0") [version]" >&2
    exit 1
fi

if [[ $# -eq 1 ]]; then
    version=${1#v}
else
    version=$("$tool" --config "$repo_root/cliff.toml" --bumped-version 2>/dev/null)
    version=${version#v}
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "refusing to release '$version': expected a bare MAJOR.MINOR.PATCH" >&2
    echo "no conventional commits since the last tag would produce no bump" >&2
    exit 1
fi

current=$(python3 - "$repo_root" <<'PY'
import re
import sys
from pathlib import Path

manifest = Path(sys.argv[1]) / "Cargo.toml"
text = manifest.read_text(encoding="utf-8")
section = re.search(r"^\[workspace\.package\]$(.*?)(?=^\[)", text, re.M | re.S)
if section is None:
    raise SystemExit("could not find [workspace.package] in Cargo.toml")
found = re.search(r'^version = "([^"]+)"$', section.group(1), re.M)
if found is None:
    raise SystemExit("could not find the workspace version in Cargo.toml")
print(found.group(1))
PY
)

if [[ "$current" == "$version" ]]; then
    echo "workspace version is already $version; nothing to prepare" >&2
    exit 1
fi

echo "preparing release: $current -> $version"

python3 - "$repo_root" "$version" <<'PY'
import re
import sys
from pathlib import Path

manifest = Path(sys.argv[1]) / "Cargo.toml"
version = sys.argv[2]
text = manifest.read_text(encoding="utf-8")


def bump(match):
    # Only the [workspace.package] version; the file also carries dependency
    # and profile sections whose version keys must not move.
    body = re.sub(r'^version = "[^"]+"$', f'version = "{version}"', match.group(1), count=1, flags=re.M)
    return match.group(0)[: match.start(1) - match.start(0)] + body


updated = re.sub(r"^\[workspace\.package\]$(.*?)(?=^\[)", bump, text, count=1, flags=re.M | re.S)
if updated == text:
    raise SystemExit("workspace version was not updated")
manifest.write_text(updated, encoding="utf-8")
PY

# Keep Cargo.lock's workspace-member versions in step with the manifest.
cargo update --workspace --offline

"$tool" --config "$repo_root/cliff.toml" --tag "v$version" --unreleased \
    --prepend "$repo_root/CHANGELOG.md"

cat <<EOF

prepared $version. Nothing has been committed, tagged or pushed.

  changed: Cargo.toml, Cargo.lock, CHANGELOG.md

next:
  1. review the changelog and the version bump
  2. run the gates from AGENTS.md against the bumped tree
  3. git commit -m "chore: release $version"
  4. git tag -a "v$version" -m "v$version"
  5. cargo xtask package        # per platform; needs the content submodule
  6. git push && git push --tags
EOF
