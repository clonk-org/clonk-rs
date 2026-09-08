#!/usr/bin/env bash
# Run the pinned oracle's Rust platform-path bridge under isolated roots and
# check what it reports (clonk-org/clonk-rs#1267).
#
# Under -DUSE_RUST_PLATFORM_PATHS=ON, C4ConfigGeneral::DeterminePaths asks this
# tree's clonk-platform for the install root, planet and system-group paths,
# the user-data, cache, logs, temp and config directories (eight
# `lc_platform_*` getters, each returning an owned string), takes the whole set
# or none of it, and asks Rust to create the user directories. The layered
# patch writes the set the oracle took to stderr (the call runs before the log
# opens) and names the getter whose null answer sent C++ back to its own
# paths. What the oracle then does with the answers is read from the config it
# saves at quit (UserPath, LogPath) and from where its log file lands.
#
# Every root is under --out. The user's real HOME, temp and user data are never
# touched: HOME, TMPDIR and every LC_*_DIR override point inside it.
#
# Build the oracle first:
#
#   parity/bridge/build-oracle-validation.sh --oracle-root <pin worktree> \
#       --with-platform-paths --build-dir build-platform
set -euo pipefail

ORACLE_ROOT=${LEGACYCLONK_ORACLE_ROOT:-}
BUILD_DIR=build-platform
REFERENCE_BUILD_DIR=build-validation
OUT=
CHECK_LEAKS=0

usage() {
	cat >&2 <<-USAGE
		usage: $0 --oracle-root <path> [--build-dir <name>] [--reference-build-dir <name>] [--out <dir>] [--leaks]

		  --oracle-root          the pin worktree the bridge oracle was built in
		                         (defaults to \$LEGACYCLONK_ORACLE_ROOT)
		  --build-dir            CMake build dir built with --with-platform-paths (default: $BUILD_DIR)
		  --reference-build-dir  a build dir WITHOUT the bridge; when present, its own
		                         user path for the same HOME is compared with Rust's
		                         (default: $REFERENCE_BUILD_DIR)
		  --out                  scratch directory for roots, homes, logs (default: a
		                         fresh directory under \$TMPDIR)
		  --leaks                run the override case under macOS \`leaks --atExit\`
		                         and fail on any leak that reaches the bridge or Rust
	USAGE
	exit 2
}

while [ $# -gt 0 ]; do
	case "$1" in
		--oracle-root) ORACLE_ROOT=${2:?}; shift 2 ;;
		--build-dir) BUILD_DIR=${2:?}; shift 2 ;;
		--reference-build-dir) REFERENCE_BUILD_DIR=${2:?}; shift 2 ;;
		--out) OUT=${2:?}; shift 2 ;;
		--leaks) CHECK_LEAKS=1; shift ;;
		-h|--help) usage ;;
		*) echo "unknown argument: $1" >&2; usage ;;
	esac
done

[ -n "$ORACLE_ROOT" ] || { echo "error: --oracle-root (or \$LEGACYCLONK_ORACLE_ROOT) is required" >&2; usage; }
ORACLE_ROOT=$(cd "$ORACLE_ROOT" && pwd)
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BUILD="$ORACLE_ROOT/$BUILD_DIR"
CLONK="$BUILD/clonk"
REFERENCE_CLONK="$ORACLE_ROOT/$REFERENCE_BUILD_DIR/clonk"
[ -x "$CLONK" ] || { echo "error: no oracle binary at $CLONK; build it with --with-platform-paths first" >&2; exit 1; }
if [ -z "$OUT" ]; then
	OUT=$(mktemp -d "${TMPDIR:-/tmp}/platform-differential.XXXXXX")
fi
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# 1. Prove which artifact is linked, the same way the other differentials do.
link_line="$BUILD/CMakeFiles/clonk.dir/link.txt"
archive=$(tr ' ' '\n' < "$link_line" | grep -E '/liblc_platform\.a$' | head -1 || true)
[ -n "$archive" ] || fail "link line $link_line does not import liblc_platform.a: the oracle was not built with --with-platform-paths"
if [ -n "$archive" ]; then
	resolved=$(cd "$(dirname "$archive")" && pwd -P)/$(basename "$archive")
	case "$resolved" in
		"$REPO_ROOT"/target/*) ;;
		*) fail "linked archive $resolved is not under $REPO_ROOT/target: the oracle linked another tree" ;;
	esac
	[ -f "$resolved" ] || fail "linked archive $resolved is missing"
fi
required_symbols="lc_platform_install_root lc_platform_planet_dir lc_platform_system_group_path lc_platform_user_data_dir lc_platform_cache_dir lc_platform_logs_dir lc_platform_temp_dir lc_platform_config_dir lc_platform_ensure_user_dirs lc_platform_string_free"
exported=$(nm "$CLONK" | awk '$2 == "T" { sub(/^_/, "", $3); print $3 }')
for symbol in $required_symbols; do
	grep -qx "$symbol" <<<"$exported" || fail "binary does not export $symbol: the clonk-platform FFI surface is not linked"
done
{
	echo "oracle binary:   $CLONK"
	echo "linked archive:  ${resolved:-<none>}"
	if [ -n "${resolved:-}" ] && [ -f "$resolved" ]; then
		echo "archive sha256:  $(shasum -a 256 "$resolved" | cut -d' ' -f1)"
	fi
	echo "port tree:       $REPO_ROOT"
	echo "port revision:   $(git -C "$REPO_ROOT" rev-parse HEAD) $(git -C "$REPO_ROOT" diff --quiet -- crates xtask && echo clean || echo dirty)"
	echo "platform symbols: $(grep -c '^lc_platform_' <<<"$exported") exported"
} | tee "$OUT/link-record.txt"

# 2. A private install root. Rust recognises a root by planet/System.c4g; the
#    pinned oracle then looks for System.c4g directly beside its executable
#    (the bridge hands it the root as ExePath and nothing else), so the root
#    carries both, and a symlink to the binary so it runs as ./clonk.
make_root() {
	local root=$1
	rm -rf "$root"; mkdir -p "$root/planet"
	ln -s "$ORACLE_ROOT/planet/System.c4g" "$root/planet/System.c4g"
	ln -s "$ORACLE_ROOT/planet/System.c4g" "$root/System.c4g"
	ln -s "$CLONK" "$root/clonk"
}
ROOT="$OUT/install"
make_root "$ROOT"

# Run the oracle from $2 with the environment assignments in $3..; $1 names the
# case. The oracle quits at once; what matters is what the bridge reported and
# what the saved config and log file say.
run_case() {
	local name=$1 root=$2; shift 2
	local log="$OUT/$name.log"
	rm -f "$OUT/$name.config"
	(
		cd "$root"
		printf '/quit\n' | env -i PATH="$PATH" "$@" ./clonk "/config:$OUT/$name.config" > "$log" 2>&1
	) || fail "$name: oracle exited with status $? (see $log)"
}

expect_line() {
	local name=$1 pattern=$2
	grep -aqF -- "$pattern" "$OUT/$name.log" || fail "$name: expected report line missing: $pattern"
}

reject_line() {
	local name=$1 pattern=$2
	! grep -aqF -- "$pattern" "$OUT/$name.log" || fail "$name: unexpected report line present: $pattern"
}

# The saved config carries the paths the oracle actually adopted; CRLF endings,
# and StdCompilerINIWrite spells every byte outside ASCII as an octal escape,
# which printf's %b turns back into the bytes.
config_value() {
	local name=$1 key=$2 raw
	raw=$(grep -a "^$key=" "$OUT/$name.config" | head -1 | sed -E "s/^$key=\"?//; s/\"?\r?$//")
	printf '%b' "$raw"
}

# 3. Every directory overridden: the bridge must hand C++ exactly the roots
#    the environment names, C++ must adopt them, and Rust must create the user
#    directories. The pinned log system opens Clonk.log in the working
#    directory, which DeterminePaths sets to ExePath, so the log file landing
#    in the install root is the proof that ExePath was taken from Rust
#    (LogPath is adopted too, but nothing at the pin reads it).
mkdir -p "$OUT/home-overrides"
run_case overrides "$ROOT" HOME="$OUT/home-overrides" TMPDIR="$OUT/tmp-overrides" \
	LC_INSTALL_ROOT="$ROOT" LC_USER_DATA_DIR="$OUT/user" LC_LOGS_DIR="$OUT/logs" \
	LC_TEMP_DIR="$OUT/temp" LC_CACHE_DIR="$OUT/cache"
expect_line overrides "Rust platform paths: install='$ROOT' planet='$ROOT/planet' system='$ROOT/planet/System.c4g' user='$OUT/user' cache='$OUT/cache' logs='$OUT/logs' temp='$OUT/temp' config='$OUT/user/Config'"
expect_line overrides "Rust platform user directories: ensured"
reject_line overrides "legacy paths used"
[ "$(config_value overrides UserPath)" = "$OUT/user" ] || fail "overrides: saved UserPath is '$(config_value overrides UserPath)', not $OUT/user"
[ "$(config_value overrides LogPath)" = "$OUT/logs/" ] || fail "overrides: saved LogPath is '$(config_value overrides LogPath)', not $OUT/logs/"
for directory in "$OUT/user" "$OUT/user/Config" "$OUT/cache" "$OUT/logs"; do
	[ -d "$directory" ] || fail "overrides: Rust did not create $directory"
done
[ -f "$ROOT/Clonk.log" ] || fail "overrides: the oracle did not open its log in the Rust install root"

# 4. Defaults: only HOME, TMPDIR and the install root are set, so the answers
#    are the port's documented policy (compat profile pres-userdata-directory):
#    user data under the product name, logs and cache beneath it, temp under
#    the product slug. The pinned oracle on its own keeps the log beside the
#    binary and temp at /tmp; that difference is the policy, not a defect, and
#    it is recorded here rather than asserted away.
HOME_DEFAULTS="$OUT/home-defaults"
mkdir -p "$HOME_DEFAULTS" "$OUT/tmp-defaults"
run_case defaults "$ROOT" HOME="$HOME_DEFAULTS" TMPDIR="$OUT/tmp-defaults" LC_INSTALL_ROOT="$ROOT"
DEFAULT_USER="$HOME_DEFAULTS/Library/Application Support/Clonk Rust"
expect_line defaults "Rust platform paths: install='$ROOT' planet='$ROOT/planet' system='$ROOT/planet/System.c4g' user='$DEFAULT_USER' cache='$DEFAULT_USER/Cache' logs='$DEFAULT_USER/Logs' temp='$OUT/tmp-defaults/clonk-rust' config='$DEFAULT_USER/Config'"
[ "$(config_value defaults UserPath)" = "$DEFAULT_USER" ] || fail "defaults: saved UserPath is '$(config_value defaults UserPath)'"
[ "$(config_value defaults LogPath)" = "$DEFAULT_USER/Logs/" ] || fail "defaults: saved LogPath is '$(config_value defaults LogPath)'"
echo "defaults: user data '$DEFAULT_USER', LogPath beneath it, temp under the product slug; the pinned oracle alone keeps LogPath beside the binary and temp at /tmp (documented policy)"

# 5. An existing LegacyClonk user directory is preferred over the product
#    name, which is where both engines agree on the same directory. With a
#    bridge-less oracle beside it, its own answer for the same HOME is read
#    from the config it saves and compared after expanding \$HOME.
HOME_LEGACY="$OUT/home-legacy"
mkdir -p "$HOME_LEGACY/Library/Application Support/LegacyClonk" "$OUT/tmp-legacy"
run_case legacy-preferred "$ROOT" HOME="$HOME_LEGACY" TMPDIR="$OUT/tmp-legacy" LC_INSTALL_ROOT="$ROOT"
LEGACY_USER="$HOME_LEGACY/Library/Application Support/LegacyClonk"
expect_line legacy-preferred "user='$LEGACY_USER'"
[ "$(config_value legacy-preferred UserPath)" = "$LEGACY_USER" ] || fail "legacy-preferred: saved UserPath is '$(config_value legacy-preferred UserPath)'"
if [ -x "$REFERENCE_CLONK" ]; then
	REF_ROOT="$OUT/install-reference"
	make_root "$REF_ROOT"
	rm -f "$REF_ROOT/clonk"; ln -s "$REFERENCE_CLONK" "$REF_ROOT/clonk"
	run_case legacy-reference "$REF_ROOT" HOME="$HOME_LEGACY" TMPDIR="$OUT/tmp-legacy"
	reference_user=$(config_value legacy-reference UserPath | sed "s|^\\\$HOME|$HOME_LEGACY|")
	[ "$reference_user" = "$LEGACY_USER" ] || fail "legacy-preferred: the bridge-less oracle's user path '$reference_user' differs from Rust's '$LEGACY_USER'"
	echo "legacy-preferred: both engines use '$LEGACY_USER'"
else
	echo "legacy-preferred: no bridge-less oracle at $REFERENCE_CLONK, C++ default not compared"
fi

# 6. A root whose name is not ASCII: the bytes must survive the C string
#    boundary in both directions and land in the saved config unchanged. A name
#    that is not valid UTF-8 cannot be created on this filesystem; that boundary
#    is the lossy conversion in the Rust getters and is documented, not run.
UNICODE_USER="$OUT/Ünïcode Räume/user"
mkdir -p "$OUT/home-unicode" "$OUT/tmp-unicode"
run_case unicode "$ROOT" HOME="$OUT/home-unicode" TMPDIR="$OUT/tmp-unicode" LC_INSTALL_ROOT="$ROOT" LC_USER_DATA_DIR="$UNICODE_USER"
expect_line unicode "user='$UNICODE_USER'"
[ "$(config_value unicode UserPath)" = "$UNICODE_USER" ] || fail "unicode: saved UserPath is '$(config_value unicode UserPath)'"
[ -d "$UNICODE_USER/Config" ] || fail "unicode: Rust did not create $UNICODE_USER/Config"

# 7. An incomplete set: one getter's answer is discarded after it was freed.
#    The pinned contract takes all eight or none, so C++ must fall back to its
#    own derivation for every path: the log lands beside the binary again and
#    the user path is C++'s literal default.
mkdir -p "$OUT/home-fault" "$OUT/tmp-fault"
run_case fault-null "$ROOT" HOME="$OUT/home-fault" TMPDIR="$OUT/tmp-fault" LC_INSTALL_ROOT="$ROOT" \
	LC_USER_DATA_DIR="$OUT/user-fault" LC_LOGS_DIR="$OUT/logs-fault" LC_RUST_PLATFORM_FAULT=lc_platform_logs_dir
expect_line fault-null "Rust platform paths unavailable (lc_platform_logs_dir returned null); legacy paths used"
reject_line fault-null "Rust platform paths: install="
reject_line fault-null "Rust platform user directories"
[ "$(config_value fault-null LogPath)" = "$ROOT/" ] || fail "fault-null: saved LogPath is '$(config_value fault-null LogPath)', not the binary's directory $ROOT/"
[ "$(config_value fault-null UserPath)" != "$OUT/user-fault" ] || fail "fault-null: C++ adopted a Rust user path from an incomplete set"
[ ! -e "$OUT/logs-fault" ] || fail "fault-null: a directory was created from an incomplete set"

# 8. Frees. Eight strings per discovery, discovered once per getter by design;
#    `leaks` proves they all come back.
if [ "$CHECK_LEAKS" = 1 ]; then
	command -v leaks >/dev/null || { echo "error: --leaks needs the macOS leaks tool" >&2; exit 1; }
	(
		cd "$ROOT"
		printf '/quit\n' | env -i PATH="$PATH" HOME="$OUT/home-overrides" TMPDIR="$OUT/tmp-overrides" \
			LC_INSTALL_ROOT="$ROOT" LC_USER_DATA_DIR="$OUT/user" LC_LOGS_DIR="$OUT/logs" \
			LC_TEMP_DIR="$OUT/temp" LC_CACHE_DIR="$OUT/cache" \
			leaks --atExit -- ./clonk "/config:$OUT/leaks.config" > "$OUT/leaks.log" 2>&1 || true
	)
	summary=$(grep -aE '^Process [0-9]+: [0-9]+ leaks? for' "$OUT/leaks.log" | head -1 || true)
	[ -n "$summary" ] || fail "leaks produced no summary (see $OUT/leaks.log)"
	if grep -aqE 'lc_platform_|RustPlatformBridge|clonk_platform' "$OUT/leaks.log"; then
		fail "a leak reaches the platform bridge (see $OUT/leaks.log)"
	fi
	echo "leaks overrides: ${summary#Process *: } (none through the bridge)"
fi

echo
if [ "$failures" -eq 0 ]; then
	echo "platform differential: all cases behaved as pinned (logs in $OUT)"
else
	echo "platform differential: $failures failure(s) (logs in $OUT)" >&2
	exit 1
fi
