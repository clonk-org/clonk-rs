#!/usr/bin/env bash
set -euo pipefail

nsis_url='https://downloads.sourceforge.net/project/nsis/NSIS%203/3.12/nsis-3.12.zip'
nsis_sha256='56581f90db321581c5381193d796fffcf2d24b2f8fed2160a6c6a3baa67f2c4f'
runner_temp=$(cygpath -u "$RUNNER_TEMP")
archive="$runner_temp/nsis-3.12.zip"
curl -fsSL --retry 5 --retry-all-errors \
  --output "$archive" "$nsis_url"
actual_sha256=$(sha256sum "$archive" | cut -d' ' -f1)
if [[ "$actual_sha256" != "$nsis_sha256" ]]; then
  echo "NSIS archive digest mismatch: expected $nsis_sha256, found $actual_sha256" >&2
  exit 1
fi
python3 -m zipfile -e "$archive" "$runner_temp"
nsis_dir="$runner_temp/nsis-3.12"
if [[ ! -x "$nsis_dir/makensis.exe" ]]; then
  echo "portable NSIS archive is missing $nsis_dir/makensis.exe" >&2
  exit 1
fi
nsis_version=$(MSYS2_ARG_CONV_EXCL='*' "$nsis_dir/makensis.exe" /VERSION | tr -d '\r')
if [[ "$nsis_version" != 'v3.12' ]]; then
  echo "expected NSIS v3.12, found: $nsis_version" >&2
  exit 1
fi
echo "$(cygpath -w "$nsis_dir")" >> "$GITHUB_PATH"
