#!/usr/bin/env bash
# Run the pinned oracle's Rust config bridge over fixture configs and check
# what it reports (clonk-org/clonk-rs#1264).
#
# The oracle under -DUSE_RUST_CONFIG=ON loads every config twice: once through
# StdCompilerINIRead into C4Config, once through this tree's clonk-core
# (`lc_config_load`). It then decompiles C4Config back to INI text, asks Rust to
# dump its own view and to diff the two (`lc_config_compare_with_dump`), and only
# when both agree does it keep the Rust handle live: the values are read back
# through `lc_config_get_value_in`, and at quit the decompiled text is pushed
# through `lc_config_replace_from_text` and written by `lc_config_save`. That
# save is the reload half of the differential: the file the Rust writer leaves
# behind must be the file the C++ writer would have left.
#
# Build the oracle first:
#
#   parity/bridge/build-oracle-validation.sh --oracle-root <pin worktree> \
#       --with-config --build-dir build-config
#
# Every run is confined to --out; the oracle gets a private HOME there and an
# explicit /config: path, so the user's real configuration is never touched.
set -euo pipefail

ORACLE_ROOT=${LEGACYCLONK_ORACLE_ROOT:-}
BUILD_DIR=build-config
REFERENCE_BUILD_DIR=build-validation
OUT=
CHECK_LEAKS=0

usage() {
	cat >&2 <<-USAGE
		usage: $0 --oracle-root <path> [--build-dir <name>] [--reference-build-dir <name>] [--out <dir>] [--leaks]

		  --oracle-root          the pin worktree the bridge oracle was built in
		                         (defaults to \$LEGACYCLONK_ORACLE_ROOT)
		  --build-dir            CMake build dir built with --with-config (default: $BUILD_DIR)
		  --reference-build-dir  a build dir WITHOUT the bridge; when present, the file
		                         the bridge oracle saves is compared with the file this
		                         one saves (default: $REFERENCE_BUILD_DIR)
		  --out                  scratch directory for fixtures, logs and HOME
		                         (default: a fresh directory under \$TMPDIR)
		  --leaks                run every fixture under macOS \`leaks --atExit\` and
		                         fail on any leak that reaches a bridge or Rust frame
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
[ -x "$CLONK" ] || { echo "error: no oracle binary at $CLONK; build it with --with-config first" >&2; exit 1; }
if [ -z "$OUT" ]; then
	OUT=$(mktemp -d "${TMPDIR:-/tmp}/config-differential.XXXXXX")
fi
mkdir -p "$OUT/home"
OUT=$(cd "$OUT" && pwd)

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# 1. Prove which artifact is linked. The pinned CMakeLists imports
#    ${CMAKE_SOURCE_DIR}/rust/target/<profile>/liblc_core.a, and the builder
#    points rust/ at this tree, so the archive in the link line must resolve
#    inside this repository and export the whole pinned config surface.
#    Fails closed when the option was left off (no symbols), when the archive
#    was taken from somewhere else (path outside this tree), or when the
#    clonk-core FFI surface was dropped from the engine archive (missing symbol).
link_line="$BUILD/CMakeFiles/clonk.dir/link.txt"
archive=$(tr ' ' '\n' < "$link_line" | grep -E '/liblc_core\.a$' | head -1 || true)
[ -n "$archive" ] || fail "link line $link_line does not import liblc_core.a: the oracle was not built with --with-config"
if [ -n "$archive" ]; then
	# The oracle reaches the archive through its rust/ symlink; resolve both ends.
	resolved=$(cd "$(dirname "$archive")" && pwd -P)/$(basename "$archive")
	case "$resolved" in
		"$REPO_ROOT"/target/*) ;;
		*) fail "linked archive $resolved is not under $REPO_ROOT/target: the oracle linked another tree" ;;
	esac
	[ -f "$resolved" ] || fail "linked archive $resolved is missing"
fi
required_symbols="lc_config_load lc_config_free lc_config_get_value lc_config_get_value_in lc_config_dump lc_config_compare_with_dump lc_config_replace_from_text lc_config_save lc_string_free"
exported=$(nm "$CLONK" | awk '$2 == "T" { sub(/^_/, "", $3); print $3 }')
for symbol in $required_symbols; do
	grep -qx "$symbol" <<<"$exported" || fail "binary does not export $symbol: the clonk-core FFI surface is not linked"
done
{
	echo "oracle binary:   $CLONK"
	echo "linked archive:  ${resolved:-<none>}"
	if [ -n "${resolved:-}" ] && [ -f "$resolved" ]; then
		echo "archive sha256:  $(shasum -a 256 "$resolved" | cut -d' ' -f1)"
	fi
	echo "port tree:       $REPO_ROOT"
	echo "port revision:   $(git -C "$REPO_ROOT" rev-parse HEAD) $(git -C "$REPO_ROOT" diff --quiet -- crates xtask && echo clean || echo dirty)"
	echo "config symbols:  $(grep -c '^lc_config_' <<<"$exported") exported"
} | tee "$OUT/link-record.txt"

# 2. The oracle refuses to start without System.c4g beside its binary.
if [ ! -e "$BUILD/System.c4g" ]; then
	ln -s "$ORACLE_ROOT/planet/System.c4g" "$BUILD/System.c4g"
fi

# Run one config through the bridge oracle; the report comes out on stderr
# because C4Config::Load runs before the log is opened (the layered patch
# writes it there). $1 = fixture name, $2 = binary. The config file is passed
# by path and is rewritten by the oracle at quit. The oracle derives its
# install root from how it was invoked and cannot open System.c4g when named
# by an absolute path, so it runs as ./clonk from its own directory.
run_oracle() {
	local name=$1 binary=$2 log="$OUT/$1.log"
	(
		cd "$(dirname "$binary")"
		printf '/quit\n' | HOME="$OUT/home" "./$(basename "$binary")" "/config:$OUT/$name.config" > "$log" 2>&1
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

# 3. Seed: an absent file. Rust has nothing to load, so the bridge must say so
#    instead of comparing an empty dump, and the oracle writes its defaults.
rm -f "$OUT/seed.config"
run_oracle seed "$CLONK"
expect_line seed "Rust config loader failed for $OUT/seed.config"
expect_line seed "Rust config dump unavailable; parity not checked"
reject_line seed "Rust config parity verified"
[ -s "$OUT/seed.config" ] || fail "seed: the oracle wrote no config"
seed="$OUT/seed.config"

# 4. Normal: the oracle's own output fed back. Both parsers see the same
#    entries, the bridge takes over, and the file Rust saves at quit must be
#    the file it was given (the writer round-trips byte for byte).
cp "$seed" "$OUT/normal.config"
run_oracle normal "$CLONK"
expect_line normal "Rust config parity verified; overrides active"
reject_line normal "Rust config diff:"
cmp -s "$seed" "$OUT/normal.config" || fail "normal: the file the Rust writer saved differs from the seed"

# 5. Edited: known keys changed to non-default values. Still parity, and the
#    edits survive the round trip through C4Config and the Rust writer.
sed -e 's/^Language="[^"]*"/Language="DE - Deutsch"/' \
	-e 's/^LanguageEx="[^"]*"/LanguageEx="DE"/' \
	-e 's/^FPS=false/FPS=true/' \
	"$seed" > "$OUT/edited.config"
cmp -s "$seed" "$OUT/edited.config" && fail "edited: the fixture edits matched nothing in the seed"
cp "$OUT/edited.config" "$OUT/edited.input"
run_oracle edited "$CLONK"
expect_line edited "Rust config parity verified; overrides active"
reject_line edited "Rust config diff:"
cmp -s "$OUT/edited.input" "$OUT/edited.config" || fail "edited: the saved file lost or changed an edit"

# 6. Unknown key: an entry C4Config has no field for. The Rust view keeps it,
#    the C++ decompile cannot, so the dumps disagree, the bridge stays off and
#    the C++ writer drops the key at quit.
# The C++ writer ends every line with CRLF; the added line matches it.
awk '{ print } /^\[Sound\]\r?$/ { print "PortOnlyKey=1\r" }' "$seed" > "$OUT/unknown-key.config"
grep -q '^PortOnlyKey=1' "$OUT/unknown-key.config" || fail "unknown-key: the seed has no [Sound] section to extend"
run_oracle unknown-key "$CLONK"
expect_line unknown-key "Rust config diff: Missing in legacy: [Sound] PortOnlyKey (rust='1')"
expect_line unknown-key "Rust config diff: Entry count differs"
expect_line unknown-key "Rust config mismatch at offset"
reject_line unknown-key "Rust config parity verified"
! grep -q '^PortOnlyKey=' "$OUT/unknown-key.config" || fail "unknown-key: the C++ writer kept the unknown key"

# 7. Missing values: known keys deleted. C4Config falls back to its defaults
#    and decompiles them; the Rust dump has no such entries, so the bridge
#    stays off and the C++ writer restores the keys.
grep -vE '^(Language|FPS)=' "$seed" > "$OUT/missing.config"
[ "$(wc -l < "$seed")" -eq "$(( $(wc -l < "$OUT/missing.config") + 2 ))" ] || fail "missing: the seed did not carry both Language and FPS"
run_oracle missing "$CLONK"
expect_line missing "Rust config diff: Missing in rust: [General] Language (legacy='')"
expect_line missing "Rust config diff: Missing in rust: [General] FPS (legacy='false')"
reject_line missing "Rust config parity verified"
grep -q '^Language=' "$OUT/missing.config" || fail "missing: the C++ writer did not restore Language"

# 8. Malformed: a non-numeric value, an unterminated string, a section header
#    without its bracket, an empty value, an empty key, a bare word, then the
#    seed again so [General] repeats. The oracle must survive it and report.
#    The report pins how the two parsers differ on garbage; the C++ side is
#    StdCompilerINIRead's name tree (StdCompiler.cpp: a header needs "[" and a
#    letter and is dropped without its "]", a line that starts with anything
#    but a letter is skipped, and the first section of a name wins). The two
#    lines where clonk-core differs (an empty key kept, a repeated section
#    merged) are clonk-org/clonk-rs#1597; change them together with that fix.
{
	printf '[General]\r\nVersion=abc\r\nLanguage="US - English\r\n[Graphics\r\nResX=\r\n=5\r\nGarbage line here\r\n'
	cat "$seed"
} > "$OUT/malformed.config"
run_oracle malformed "$CLONK"
expect_line malformed "Rust config diff: Missing in legacy: [General] Version (rust='abc')"
expect_line malformed "Rust config diff: Missing in legacy: [General] ResX (rust='')"
expect_line malformed "Rust config diff: Missing in legacy: [General]  (rust='5')"
expect_line malformed "Rust config diff: Value mismatch for [General] LanguageEx (rust='US', legacy='')"
reject_line malformed "Rust config parity verified"
[ -s "$OUT/malformed.config" ] || fail "malformed: the oracle wrote no config back"

# 9. Save differential against a bridge-less oracle, when one is built: the
#    file the Rust writer saved must match the C++ writer's own output for the
#    same input. LogPath names each binary's directory and is the one line
#    allowed to differ.
if [ -x "$REFERENCE_CLONK" ]; then
	if [ ! -e "$(dirname "$REFERENCE_CLONK")/System.c4g" ]; then
		ln -s "$ORACLE_ROOT/planet/System.c4g" "$(dirname "$REFERENCE_CLONK")/System.c4g"
	fi
	for name in normal edited; do
		cp "$OUT/$name.config" "$OUT/$name-reference.config"
		run_oracle "$name-reference" "$REFERENCE_CLONK"
		if ! diff <(grep -v '^LogPath=' "$OUT/$name.config") <(grep -v '^LogPath=' "$OUT/$name-reference.config") > "$OUT/$name-save.diff"; then
			fail "$name: the Rust-saved file differs from the C++-saved file (see $OUT/$name-save.diff)"
		fi
	done
	echo "save differential: Rust writer matches the C++ writer on normal and edited"
else
	echo "save differential: skipped, no bridge-less oracle at $REFERENCE_CLONK"
fi

# 10. Frees. Every returned string and the handle are freed by the pinned
#     bridge; `leaks` proves it. AppKit leaks a few Foundation objects at exit
#     on its own, so only a leak whose stack reaches a bridge or Rust frame
#     counts.
if [ "$CHECK_LEAKS" = 1 ]; then
	command -v leaks >/dev/null || { echo "error: --leaks needs the macOS leaks tool" >&2; exit 1; }
	for name in normal unknown-key missing malformed; do
		cp "$OUT/$name.config" "$OUT/$name-leaks.config" 2>/dev/null || cp "$seed" "$OUT/$name-leaks.config"
		(
			cd "$BUILD"
			printf '/quit\n' | HOME="$OUT/home" leaks --atExit -- ./clonk "/config:$OUT/$name-leaks.config" > "$OUT/$name-leaks.log" 2>&1 || true
		)
		summary=$(grep -aE '^Process [0-9]+: [0-9]+ leaks? for' "$OUT/$name-leaks.log" | head -1 || true)
		[ -n "$summary" ] || fail "$name: leaks produced no summary (see $OUT/$name-leaks.log)"
		if grep -aqE 'lc_config_|lc_string_|RustConfigBridge|clonk_core|std_config' "$OUT/$name-leaks.log"; then
			fail "$name: a leak reaches the config bridge (see $OUT/$name-leaks.log)"
		fi
		echo "leaks $name: ${summary#Process *: } (none through the bridge)"
	done
fi

echo
if [ "$failures" -eq 0 ]; then
	echo "config differential: all fixtures behaved as pinned (logs in $OUT)"
else
	echo "config differential: $failures failure(s) (logs in $OUT)" >&2
	exit 1
fi
