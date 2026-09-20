#!/usr/bin/env bash
# Run the GUI bridge differential (clonk-org/clonk-rs#1266).
#
# The pinned oracle links the Rust GUI wrapper under -DUSE_RUST_GUI_VALIDATION
# but never calls it, so an oracle build is ABI evidence only: it proves the
# option compiles the pinned wrapper and links this tree's archive. The
# behavioural half is a focused C++ harness (gui-harness.cpp) that drives the
# pinned wrapper (src/rust/RustGuiBridge.* from the oracle worktree) over the
# C ABI through a scripted tree, layout, render export, pointer and key events,
# reset, move ownership and frees, and prints one canonical dump; the Rust side
# (crates/clonk-gui/examples/bridge_scenario.rs) runs the same script through
# the safe API and prints the same lines. The two dumps must be identical, and
# perturbing either side must break that.
#
# None of this claims the unmodified C++ application routes its native menus
# through the wrapper. It does not; nothing in the pinned executable calls it.
#
# Build the oracle first:
#
#   parity/bridge/build-oracle-validation.sh --oracle-root <pin worktree> \
#       --with-gui-validation --build-dir build-gui
set -euo pipefail

ORACLE_ROOT=${LEGACYCLONK_ORACLE_ROOT:-}
BUILD_DIR=build-gui
OUT=
CHECK_LEAKS=0

usage() {
	cat >&2 <<-USAGE
		usage: $0 --oracle-root <path> [--build-dir <name>] [--out <dir>] [--leaks]

		  --oracle-root  the pin worktree the bridge oracle was built in
		                 (defaults to \$LEGACYCLONK_ORACLE_ROOT)
		  --build-dir    CMake build dir built with --with-gui-validation (default: $BUILD_DIR)
		  --out          scratch directory for the harness and dumps (default: a
		                 fresh directory under \$TMPDIR)
		  --leaks        run the harness under macOS \`leaks --atExit\` and fail on
		                 any leak that reaches the wrapper or a Rust frame
	USAGE
	exit 2
}

while [ $# -gt 0 ]; do
	case "$1" in
		--oracle-root) ORACLE_ROOT=${2:?}; shift 2 ;;
		--build-dir) BUILD_DIR=${2:?}; shift 2 ;;
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
[ -x "$CLONK" ] || { echo "error: no oracle binary at $CLONK; build it with --with-gui-validation first" >&2; exit 1; }
if [ -z "$OUT" ]; then
	OUT=$(mktemp -d "${TMPDIR:-/tmp}/gui-differential.XXXXXX")
fi
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# 1. Prove which artifact the oracle linked and that it exports the whole
#    pinned GUI surface; the harness then links the very same archive.
link_line="$BUILD/CMakeFiles/clonk.dir/link.txt"
archive=$(tr ' ' '\n' < "$link_line" | grep -E '/liblc_gui\.a$' | head -1 || true)
[ -n "$archive" ] || fail "link line $link_line does not import liblc_gui.a: the oracle was not built with --with-gui-validation"
resolved=
if [ -n "$archive" ]; then
	resolved=$(cd "$(dirname "$archive")" && pwd -P)/$(basename "$archive")
	case "$resolved" in
		"$REPO_ROOT"/target/*) ;;
		*) fail "linked archive $resolved is not under $REPO_ROOT/target: the oracle linked another tree" ;;
	esac
	[ -f "$resolved" ] || fail "linked archive $resolved is missing"
fi
required_symbols="lc_gui_create lc_gui_free lc_gui_reset lc_gui_root lc_gui_add_column lc_gui_add_label lc_gui_add_button lc_gui_layout lc_gui_layout_unbounded lc_gui_render lc_gui_render_data lc_gui_render_free lc_gui_pointer_event lc_gui_key_event lc_gui_event_result_view lc_gui_event_result_free"
# Nothing in the oracle calls the wrapper, so the linker drops it and the
# surface it pulled in; the binary cannot show the symbols. What survives is
# the wrapper object CMake compiled against the vendored header, whose
# undefined references name the surface, and the archive it was resolved from.
wrapper_object="$BUILD/CMakeFiles/clonk.dir/src/rust/RustGuiBridge.cpp.o"
[ -f "$wrapper_object" ] || fail "the oracle build has no compiled wrapper at $wrapper_object: the option was not on"
# nm exits non-zero on the archive's bitcode members while still listing the
# rest, so its status is not the signal; the symbol checks below are.
referenced=$( { [ -f "$wrapper_object" ] && nm -u "$wrapper_object" 2>/dev/null || true; } | awk '{ sub(/^_/, "", $NF); print $NF }')
archived=$( { [ -n "${resolved:-}" ] && nm "$resolved" 2>/dev/null || true; } | awk '$2 == "T" { sub(/^_/, "", $3); print $3 }')
for symbol in $required_symbols; do
	grep -qx "$symbol" <<<"$referenced" || fail "the compiled wrapper does not reference $symbol: the pinned wrapper drifted from the header"
	grep -qx "$symbol" <<<"$archived" || fail "the linked archive does not export $symbol: the clonk-gui FFI surface is not in it"
done
in_binary=$(nm "$CLONK" 2>/dev/null | awk '$2 == "T" { sub(/^_/, "", $3); print $3 }' | grep -c '^lc_gui_' || true)
{
	echo "oracle binary:   $CLONK"
	echo "linked archive:  ${resolved:-<none>}"
	if [ -n "${resolved:-}" ] && [ -f "$resolved" ]; then
		echo "archive sha256:  $(shasum -a 256 "$resolved" | cut -d' ' -f1)"
	fi
	echo "port tree:       $REPO_ROOT"
	echo "port revision:   $(git -C "$REPO_ROOT" rev-parse HEAD) $(git -C "$REPO_ROOT" diff --quiet -- crates xtask && echo clean || echo dirty)"
	echo "gui symbols:     $(grep -c '^lc_gui_' <<<"$referenced") referenced by the compiled wrapper, $(grep -c '^lc_gui_' <<<"$archived") exported by the archive, $in_binary kept in the binary (nothing calls the wrapper)"
} | tee "$OUT/link-record.txt"
[ -n "${resolved:-}" ] || { echo "no archive to drive; stopping" >&2; exit 1; }

# 2. The harness: the pinned wrapper compiled from the oracle worktree, the
#    vendored header, and the archive the oracle linked.
wrapper="$ORACLE_ROOT/src/rust/RustGuiBridge.cpp"
[ -f "$wrapper" ] || { echo "error: no pinned wrapper at $wrapper" >&2; exit 1; }
if ! xcrun clang++ -std=c++20 -DUSE_RUST_GUI_VALIDATION \
	-I "$REPO_ROOT/parity/bridge" -I "$ORACLE_ROOT/src/rust" \
	-o "$OUT/gui-harness" "$REPO_ROOT/parity/bridge/gui-harness.cpp" "$wrapper" "$resolved" \
	-framework CoreFoundation -framework Security -liconv -lz > "$OUT/harness-build.log" 2>&1; then
	echo "error: the harness did not build (see $OUT/harness-build.log)" >&2
	exit 1
fi

# 3. The Rust side, built once with the bridge feature so it measures with the
#    bridge's font.
rust_side() {
	(cd "$REPO_ROOT" && cargo run -q --release -p clonk-gui --features ffi --example bridge_scenario -- "$@")
}

rust_side > "$OUT/rust.dump" 2> "$OUT/rust.err" || fail "the Rust scenario failed (see $OUT/rust.err)"
"$OUT/gui-harness" > "$OUT/cpp.dump" 2> "$OUT/cpp.err" || fail "the harness failed (see $OUT/cpp.err)"

# 4. Agreement: byte for byte, and both dumps must actually carry the script.
if diff "$OUT/rust.dump" "$OUT/cpp.dump" > "$OUT/agree.diff"; then
	echo "agreement: $(wc -l < "$OUT/rust.dump" | tr -d ' ') identical lines"
else
	fail "the Rust and C++ dumps differ (see $OUT/agree.diff)"
fi
for line in "ids root=" "render bounded count=" 'text="Press me"' "event up captured=" ":activate]" "render unbounded count=" "render after-reset count=0" "root-after-reset="; do
	grep -qF -- "$line" "$OUT/cpp.dump" || fail "the harness dump lacks '$line'"
done

# 5. Perturbing either side must break the agreement, or the comparison is
#    not live.
rust_side --perturb > "$OUT/rust-perturbed.dump" 2>/dev/null || fail "the perturbed Rust scenario failed"
if diff -q "$OUT/rust-perturbed.dump" "$OUT/cpp.dump" > /dev/null; then
	fail "perturbing the Rust side left the dumps identical"
else
	echo "perturbed Rust side: dumps differ, as they must"
fi
"$OUT/gui-harness" --perturb > "$OUT/cpp-perturbed.dump" 2>/dev/null || fail "the perturbed harness failed"
if diff -q "$OUT/rust.dump" "$OUT/cpp-perturbed.dump" > /dev/null; then
	fail "perturbing the C++ side left the dumps identical"
else
	echo "perturbed C++ side: dumps differ, as they must"
fi

# 6. Frees: every handle the wrapper takes (gui, render, event result) is given
#    back, including across the move.
if [ "$CHECK_LEAKS" = 1 ]; then
	command -v leaks >/dev/null || { echo "error: --leaks needs the macOS leaks tool" >&2; exit 1; }
	leaks --atExit -- "$OUT/gui-harness" > "$OUT/leaks.log" 2>&1 || true
	summary=$(grep -aE '^Process [0-9]+: [0-9]+ leaks? for' "$OUT/leaks.log" | head -1 || true)
	[ -n "$summary" ] || fail "leaks produced no summary (see $OUT/leaks.log)"
	if grep -aqE 'lc_gui_|RustGuiBridge|clonk_gui' "$OUT/leaks.log"; then
		fail "a leak reaches the GUI bridge (see $OUT/leaks.log)"
	fi
	echo "leaks harness: ${summary#Process *: } (none through the bridge)"
fi

echo
if [ "$failures" -eq 0 ]; then
	echo "gui differential: the pinned wrapper and the safe API agree (logs in $OUT)"
else
	echo "gui differential: $failures failure(s) (logs in $OUT)" >&2
	exit 1
fi
