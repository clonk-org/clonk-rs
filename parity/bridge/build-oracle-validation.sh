#!/usr/bin/env bash
# Build the pinned LegacyClonk oracle with -DUSE_RUST_ENGINE_VALIDATION=ON so
# its shadow-diff bridge links *this* tree instead of the Rust snapshot bundled
# at the pin (clonk-org/clonk-rs#585).
#
# This MUTATES the oracle checkout you point it at, so it insists on a
# dedicated worktree parked exactly at the pin. Make one with:
#
#   git -C <oracle-repo> worktree add <path> 7d43b47b7d789b533f32d005e64596e0a07019cd
#
# Everything it changes is either recoverable with `git checkout --` in that
# worktree or confined to untracked build output.
set -euo pipefail

PIN=7d43b47b7d789b533f32d005e64596e0a07019cd
PROFILE=release
BUILD_DIR=build-validation
ORACLE_ROOT=${LEGACYCLONK_ORACLE_ROOT:-}
USE_RUST_CONFIG=OFF
USE_RUST_GROUP_VALIDATION=OFF
USE_RUST_PLATFORM_PATHS=OFF

usage() {
	cat >&2 <<-USAGE
		usage: $0 --oracle-root <path> [--profile release|debug] [--build-dir <name>]

		  --oracle-root  a git worktree of the oracle parked at $PIN
		                 (defaults to \$LEGACYCLONK_ORACLE_ROOT)
		  --profile      cargo/CMake profile to build (default: $PROFILE)
		  --build-dir    CMake build directory inside the oracle (default: $BUILD_DIR)
		  --with-config            also enable USE_RUST_CONFIG (clonk-org/clonk-rs#1264)
		  --with-group-validation  also enable USE_RUST_GROUP_VALIDATION (clonk-org/clonk-rs#1265)
		  --with-platform-paths    also enable USE_RUST_PLATFORM_PATHS (clonk-org/clonk-rs#1267)
	USAGE
	exit 2
}

while [ $# -gt 0 ]; do
	case "$1" in
		--oracle-root) ORACLE_ROOT=${2:?}; shift 2 ;;
		--profile) PROFILE=${2:?}; shift 2 ;;
		--build-dir) BUILD_DIR=${2:?}; shift 2 ;;
		--with-config) USE_RUST_CONFIG=ON; shift ;;
		--with-group-validation) USE_RUST_GROUP_VALIDATION=ON; shift ;;
		--with-platform-paths) USE_RUST_PLATFORM_PATHS=ON; shift ;;
		-h|--help) usage ;;
		*) echo "unknown argument: $1" >&2; usage ;;
	esac
done

[ -n "$ORACLE_ROOT" ] || { echo "error: --oracle-root (or \$LEGACYCLONK_ORACLE_ROOT) is required" >&2; usage; }
[ -d "$ORACLE_ROOT" ] || { echo "error: no such directory: $ORACLE_ROOT" >&2; exit 1; }

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ORACLE_ROOT=$(cd "$ORACLE_ROOT" && pwd)
ORACLE_PATCH="$REPO_ROOT/parity/bridge/oracle-weather.patch"

# The oracle must be at the pin. A drifted checkout is a *wrong* oracle, not a
# slightly stale one, so this is a hard error rather than a warning.
head=$(git -C "$ORACLE_ROOT" rev-parse HEAD)
if [ "$head" != "$PIN" ]; then
	echo "error: $ORACLE_ROOT is at $head, not the pin $PIN" >&2
	exit 1
fi

# Keep the oracle commit fixed while layering the reviewed weather transport
# instrumentation needed by this checkout. Refuse a partial or drifted patch:
# neither applying nor cleanly reversing the complete patch is trustworthy.
if [ ! -f "$ORACLE_PATCH" ]; then
	echo "error: missing oracle weather patch: $ORACLE_PATCH" >&2
	exit 1
fi
if git -C "$ORACLE_ROOT" apply --check "$ORACLE_PATCH" >/dev/null 2>&1; then
	git -C "$ORACLE_ROOT" apply "$ORACLE_PATCH"
	echo "==> applied the oracle weather instrumentation patch"
elif git -C "$ORACLE_ROOT" apply --reverse --check "$ORACLE_PATCH" >/dev/null 2>&1; then
	echo "==> oracle weather instrumentation patch already applied"
else
	echo "error: oracle weather patch is partially applied or does not match $PIN" >&2
	exit 1
fi

# The config bridge's C++ never compiled at the pin: two assigners in
# src/C4Config.cpp take the wrong types (clonk-org/clonk-rs#1264). The fix is
# layered the same way as the weather instrumentation, and only when the
# option that compiles that code is requested.
CONFIG_PATCH="$REPO_ROOT/parity/bridge/oracle-config-bridge.patch"
if [ "$USE_RUST_CONFIG" = ON ]; then
	if git -C "$ORACLE_ROOT" apply --check "$CONFIG_PATCH" >/dev/null 2>&1; then
		git -C "$ORACLE_ROOT" apply "$CONFIG_PATCH"
		echo "==> applied the oracle config-bridge compile fix"
	elif git -C "$ORACLE_ROOT" apply --reverse --check "$CONFIG_PATCH" >/dev/null 2>&1; then
		echo "==> oracle config-bridge compile fix already applied"
	else
		echo "error: oracle config-bridge patch is partially applied or does not match $PIN" >&2
		exit 1
	fi
fi
echo "==> oracle   $ORACLE_ROOT (at the pin)"
echo "==> port     $REPO_ROOT"
echo "==> profile  $PROFILE"

# 1. The fmt trap. The pin vendors fmt 11 headers in deps/include, which is on
#    the include path for zlib/curl, while find_package(fmt) links Homebrew's
#    fmt 12 -- yielding hundreds of undefined fmt::v11 symbols at link time.
if [ -d "$ORACLE_ROOT/deps/include/fmt" ]; then
	mv "$ORACLE_ROOT/deps/include/fmt" "$ORACLE_ROOT/deps/include/fmt.v11-shadowed"
	echo "==> shadowed the vendored fmt 11 headers"
fi

# 2. The reason this option has never been usable: the pinned CMakeLists carries
#    a literal backspace (0x08) glued to the clonk_engine_static target name in
#    all three places it appears. CMake then rejects the name as invalid while
#    *printing* the clean one, so it reads like a missing-artifact path problem.
if LC_ALL=C grep -q $'clonk_engine_static\b' "$ORACLE_ROOT/CMakeLists.txt"; then
	perl -i -pe 's/clonk_engine_static\x08/clonk_engine_static/g' "$ORACLE_ROOT/CMakeLists.txt"
	echo "==> stripped the stray 0x08 bytes from clonk_engine_static"
fi

# 3. RUST_INCLUDE_DIR is hardcoded to <oracle>/rust/include, and step 4 points
#    <oracle>/rust at this tree -- so the header has to be reachable there.
#    Untracked on purpose; .gitignore covers it. The config, group and
#    platform headers are the contracts the pinned C++ compiles against
#    unchanged, so a copy that has drifted from the pin's own rust/include/ is
#    a hard error: the bridge would then link a surface the oracle never
#    called. The engine header is exempt because it deliberately extends the
#    pin with the weather transport that the layered weather patch consumes.
mkdir -p "$REPO_ROOT/include"
for header in lc_config_ffi.h lc_group_ffi.h lc_platform_ffi.h; do
	if ! git -C "$ORACLE_ROOT" show "${PIN}:rust/include/$header" | cmp -s - "$REPO_ROOT/parity/bridge/$header"; then
		echo "error: parity/bridge/$header differs from rust/include/$header at the pin $PIN" >&2
		exit 1
	fi
done
echo "==> vendored config, group and platform headers match the pin"
for header in lc_engine_ffi.h lc_config_ffi.h lc_group_ffi.h lc_platform_ffi.h; do
	ln -sfn "../parity/bridge/$header" "$REPO_ROOT/include/$header"
done

# 4. Point the oracle's Rust tree at this checkout. Every path in CMakeLists is
#    hardcoded to ${CMAKE_SOURCE_DIR}/rust with no override variable, and
#    add_dependencies(clonk rust_build) runs `cargo xtask ffi` in that
#    directory -- so a symlink is what makes the oracle build *your* tree.
#    A link left behind by an earlier session points at a worktree that may
#    no longer exist, so an existing link is repointed unless it already names
#    this tree.
if [ ! -L "$ORACLE_ROOT/rust" ] || [ "$(readlink "$ORACLE_ROOT/rust")" != "$REPO_ROOT" ]; then
	rm -rf "$ORACLE_ROOT/rust"
	ln -s "$REPO_ROOT" "$ORACLE_ROOT/rust"
	echo "==> pointed $ORACLE_ROOT/rust at this tree"
fi

sdk=$(xcrun --show-sdk-path)
cmake -S "$ORACLE_ROOT" -B "$ORACLE_ROOT/$BUILD_DIR" \
	-DCMAKE_BUILD_TYPE="$(tr '[:lower:]' '[:upper:]' <<<"${PROFILE:0:1}")${PROFILE:1}" \
	-DUSE_RUST_ENGINE_VALIDATION=ON -DUSE_CONSOLE=ON \
	-DUSE_RUST_CONFIG="$USE_RUST_CONFIG" \
	-DUSE_RUST_GROUP_VALIDATION="$USE_RUST_GROUP_VALIDATION" \
	-DUSE_RUST_PLATFORM_PATHS="$USE_RUST_PLATFORM_PATHS" \
	-DUSE_MINIUPNPC=OFF -DUSE_TESTS=OFF \
	-DCMAKE_PREFIX_PATH=/opt/homebrew \
	-DZLIB_INCLUDE_DIR="$ORACLE_ROOT/deps/include" \
	-DZLIB_LIBRARY_RELEASE="$sdk/usr/lib/libz.tbd" \
	-DCURL_NO_CURL_CMAKE=ON \
	-DCURL_INCLUDE_DIR="$ORACLE_ROOT/deps/include" \
	-DCURL_LIBRARY="$sdk/usr/lib/libcurl.tbd" >/dev/null

cmake --build "$ORACLE_ROOT/$BUILD_DIR" --target clonk -j 8

cat <<-DONE

	built: $ORACLE_ROOT/$BUILD_DIR/clonk

	Link the game data next to it (Graphics.c4g and System.c4g from the oracle's
	own planet/, the rest from content/) and add a player .c4p, then run it
	ARMED -- with no LC_RUST_ENGINE_* variable set, EnsureInitialised() sets
	g_disabled and OnFrame compares nothing, which looks exactly like a clean
	diff:

	    LC_RUST_ENGINE_RUNTIME=1 ./clonk        # live lockstep diff
	    LC_RUST_ENGINE_RECORD=<path> ./clonk    # dump C++ snapshots as JSON

	Divergences are reported as "Rust runtime parity mismatch: ...".
DONE
if [ "$USE_RUST_CONFIG" = ON ]; then
	cat <<-CONFIG

		The config bridge is linked. Run its differential (fixtures, save
		round-trip, frees) with:

		    parity/bridge/run-config-differential.sh --oracle-root "$ORACLE_ROOT" --build-dir "$BUILD_DIR"
	CONFIG
fi
