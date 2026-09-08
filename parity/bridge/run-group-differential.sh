#!/usr/bin/env bash
# Run the pinned oracle's Rust group-validation bridge over fixture groups and
# check what it reports (clonk-org/clonk-rs#1265).
#
# Under -DUSE_RUST_GROUP_VALIDATION=ON every top-level group C4Group opens
# (packed file or folder) is opened again through this tree's clonk-resources
# (`lc_group_open`, `lc_group_entries`), and the two entry lists are compared by
# canonical name, size and type; the pinned bridge reports only disagreement.
# The layered patch adds the agreement line the differential keys on, a fault
# hook (LC_RUST_GROUP_FAULT) that perturbs the C++ list so each report path is
# proven, and a deep mode (LC_RUST_GROUP_DEEP) that also reads every file
# through both engines, asks `lc_group_exists` for every entry, and compares
# maker and root, freeing every buffer and string it was handed.
#
# The oracle opens System.c4g from its own directory, so each fixture is placed
# as System.c4g inside a private install root that holds a symlink to the
# binary. The oracle then fails to find its scripts and exits non-zero, which is
# expected: validation runs at open, before any content is read.
#
# Build the oracle first:
#
#   parity/bridge/build-oracle-validation.sh --oracle-root <pin worktree> \
#       --with-group-validation --build-dir build-config
set -euo pipefail

ORACLE_ROOT=${LEGACYCLONK_ORACLE_ROOT:-}
BUILD_DIR=build-config
OUT=
CHECK_LEAKS=0

usage() {
	cat >&2 <<-USAGE
		usage: $0 --oracle-root <path> [--build-dir <name>] [--out <dir>] [--leaks]

		  --oracle-root  the pin worktree the bridge oracle was built in
		                 (defaults to \$LEGACYCLONK_ORACLE_ROOT)
		  --build-dir    CMake build dir built with --with-group-validation (default: $BUILD_DIR)
		  --out          scratch directory for fixtures, roots, logs and HOME
		                 (default: a fresh directory under \$TMPDIR)
		  --leaks        run the fixtures under macOS \`leaks --atExit\` and fail on
		                 any leak that reaches the bridge or a Rust frame
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
C4GROUP="$BUILD/c4group"
[ -x "$CLONK" ] || { echo "error: no oracle binary at $CLONK; build it with --with-group-validation first" >&2; exit 1; }
if [ -z "$OUT" ]; then
	OUT=$(mktemp -d "${TMPDIR:-/tmp}/group-differential.XXXXXX")
fi
mkdir -p "$OUT/home"
OUT=$(cd "$OUT" && pwd)

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# 1. Prove which artifact is linked, the same way the config differential does:
#    the imported liblc_resources.a must resolve under this tree's target/ and
#    the binary must export the whole pinned group surface.
link_line="$BUILD/CMakeFiles/clonk.dir/link.txt"
archive=$(tr ' ' '\n' < "$link_line" | grep -E '/liblc_resources\.a$' | head -1 || true)
[ -n "$archive" ] || fail "link line $link_line does not import liblc_resources.a: the oracle was not built with --with-group-validation"
if [ -n "$archive" ]; then
	resolved=$(cd "$(dirname "$archive")" && pwd -P)/$(basename "$archive")
	case "$resolved" in
		"$REPO_ROOT"/target/*) ;;
		*) fail "linked archive $resolved is not under $REPO_ROOT/target: the oracle linked another tree" ;;
	esac
	[ -f "$resolved" ] || fail "linked archive $resolved is missing"
fi
required_symbols="lc_group_open lc_group_free lc_group_entries lc_group_entries_free lc_group_read_file lc_group_buffer_free lc_group_exists lc_group_maker lc_group_root lc_group_string_free"
exported=$(nm "$CLONK" | awk '$2 == "T" { sub(/^_/, "", $3); print $3 }')
for symbol in $required_symbols; do
	grep -qx "$symbol" <<<"$exported" || fail "binary does not export $symbol: the clonk-resources FFI surface is not linked"
done
{
	echo "oracle binary:   $CLONK"
	echo "linked archive:  ${resolved:-<none>}"
	if [ -n "${resolved:-}" ] && [ -f "$resolved" ]; then
		echo "archive sha256:  $(shasum -a 256 "$resolved" | cut -d' ' -f1)"
	fi
	echo "port tree:       $REPO_ROOT"
	echo "port revision:   $(git -C "$REPO_ROOT" rev-parse HEAD) $(git -C "$REPO_ROOT" diff --quiet -- crates xtask && echo clean || echo dirty)"
	echo "group symbols:   $(grep -c '^lc_group_' <<<"$exported") exported"
} | tee "$OUT/link-record.txt"

# 2. The fixture folder: plain files of several sizes including an empty one,
#    a nested directory, a child group folder with its own entry, and names
#    with a space and mixed case. C4Group ignores dotfiles and Thumbs.db
#    (C4Group_TestIgnore), so one of each is planted to prove both readers
#    skip them.
FIXTURE="$OUT/Fixture.c4g"
rm -rf "$FIXTURE"
mkdir -p "$FIXTURE/Nested/Deeper" "$FIXTURE/Child.c4g"
printf 'alpha' > "$FIXTURE/Alpha.txt"
: > "$FIXTURE/Empty.bin"
head -c 70000 /dev/zero | tr '\0' 'x' > "$FIXTURE/Large.bin"
printf 'with space' > "$FIXTURE/With Space.txt"
printf 'MixedCase' > "$FIXTURE/MixedCase.TXT"
printf 'inner' > "$FIXTURE/Nested/Inner.txt"
printf 'deeper' > "$FIXTURE/Nested/Deeper/Deepest.txt"
printf 'child' > "$FIXTURE/Child.c4g/InChild.txt"
printf 'hidden' > "$FIXTURE/.hidden"
printf 'thumbs' > "$FIXTURE/Thumbs.db"
# Seven visible top-level entries: 5 files, Nested and Child.c4g. The two
# ignored names must not be counted by either side.
VISIBLE_ENTRIES=7

# 3. Packed forms of the same fixture: one by the oracle's own c4group, one by
#    this tree's clonk-c4group. Both must open through both readers; the port's
#    pack read by C4Group is a differential of the writer as well.
pack_with_oracle() {
	local source=$1 target=$2
	rm -rf "$target"
	cp -R "$source" "$target"
	(cd "$(dirname "$target")" && "$C4GROUP" "$(basename "$target")" -p > /dev/null 2>&1) || return 1
	[ -f "$target" ]
}
pack_with_port() {
	local source=$1 target=$2
	rm -rf "$target"
	cp -R "$source" "$target"
	(cd "$REPO_ROOT" && cargo run -q --release -p clonk-c4group -- "$target" -p > /dev/null 2>&1) || return 1
	[ -f "$target" ]
}
if [ -x "$C4GROUP" ]; then
	pack_with_oracle "$FIXTURE" "$OUT/packed-by-oracle.c4g" || fail "the oracle's c4group could not pack the fixture"
else
	echo "note: no c4group beside the oracle (cmake --build $BUILD --target c4group); the oracle-packed case is skipped"
fi
pack_with_port "$FIXTURE" "$OUT/packed-by-port.c4g" || fail "clonk-c4group could not pack the fixture"

# Run the oracle with $2 installed as its System.c4g in a private root. $1 =
# case name, $2 = fixture path (folder or packed file), $3.. = environment
# assignments for the bridge. The oracle exits non-zero once it finds no
# scripts; that is after validation and is not a failure here.
run_case() {
	local name=$1 fixture=$2; shift 2
	local root="$OUT/root-$name" log="$OUT/$name.log"
	rm -rf "$root"; mkdir -p "$root"
	ln -s "$CLONK" "$root/clonk"
	if [ -d "$fixture" ]; then
		ln -s "$fixture" "$root/System.c4g"
	else
		cp "$fixture" "$root/System.c4g"
	fi
	(
		cd "$root"
		printf '/quit\n' | env HOME="$OUT/home" "$@" ./clonk "/config:$OUT/$name.config" > "$log" 2>&1 || true
	)
	grep -aq 'Rust group validation' "$log" || fail "$name: the bridge reported nothing (see $log)"
}

expect_line() {
	local name=$1 pattern=$2
	grep -aqF -- "$pattern" "$OUT/$name.log" || fail "$name: expected report line missing: $pattern"
}

reject_line() {
	local name=$1 pattern=$2
	! grep -aqF -- "$pattern" "$OUT/$name.log" || fail "$name: unexpected report line present: $pattern"
}

# 4. Agreement: the folder, both packed forms, and the oracle's real system
#    folder. The bridge sees the same entries through both readers, and in deep
#    mode the same bytes, the same existence answers, and the same maker and
#    root, on every one of them.
run_case folder "$FIXTURE" LC_RUST_GROUP_DEEP=1
expect_line folder "Rust group validation System.c4g: $VISIBLE_ENTRIES entries agree"
expect_line folder "Rust group validation System.c4g: deep check agrees"
reject_line folder "missing from Rust view"
reject_line folder "additional entries reported by Rust"

run_case packed-by-port "$OUT/packed-by-port.c4g" LC_RUST_GROUP_DEEP=1
expect_line packed-by-port "Rust group validation System.c4g: $VISIBLE_ENTRIES entries agree"
expect_line packed-by-port "Rust group validation System.c4g: deep check agrees"

if [ -f "$OUT/packed-by-oracle.c4g" ]; then
	run_case packed-by-oracle "$OUT/packed-by-oracle.c4g" LC_RUST_GROUP_DEEP=1
	expect_line packed-by-oracle "Rust group validation System.c4g: $VISIBLE_ENTRIES entries agree"
	expect_line packed-by-oracle "Rust group validation System.c4g: deep check agrees"
fi

run_case real-system "$ORACLE_ROOT/planet/System.c4g" LC_RUST_GROUP_DEEP=1
expect_line real-system "entries agree"
expect_line real-system "deep check agrees"

# 5. Faults: the hook perturbs the C++ list before the comparison, one way per
#    run, so each report path is shown to fire and to name the entry. A type
#    mismatch is only checked on packed groups, so that fault runs on one.
run_case fault-missing "$FIXTURE" LC_RUST_GROUP_FAULT=missing
expect_line fault-missing "Rust group validation System.c4g: entries missing from Rust view: PhantomEntry.txt"
reject_line fault-missing "entries agree"

run_case fault-additional "$FIXTURE" LC_RUST_GROUP_FAULT=additional
expect_line fault-additional "Rust group validation System.c4g: additional entries reported by Rust: "
reject_line fault-additional "entries agree"

run_case fault-size "$FIXTURE" LC_RUST_GROUP_FAULT=size
expect_line fault-size "Rust group validation System.c4g: size mismatch for entries: "
reject_line fault-size "entries agree"

run_case fault-type "$OUT/packed-by-port.c4g" LC_RUST_GROUP_FAULT=type
expect_line fault-type "Rust group validation System.c4g: entry type mismatch for: "
reject_line fault-type "entries agree"

run_case fault-read "$FIXTURE" LC_RUST_GROUP_DEEP=1 LC_RUST_GROUP_FAULT=read
expect_line fault-read "Rust group validation System.c4g: read mismatch for entries: "
reject_line fault-read "deep check agrees"

# 6. A group Rust cannot open: the bridge must say so rather than compare
#    against nothing. C4Group has to open the group for the bridge to run at
#    all, so the fault hook hands Rust a path that is not there.
run_case fault-open "$FIXTURE" LC_RUST_GROUP_FAULT=open
expect_line fault-open "Rust group validation failed: could not open"
reject_line fault-open "entries agree"

# 7. Frees. The pinned bridge owns every handle, entry array, buffer and string
#    it is handed; `leaks` proves it on the folder and on a packed group, in
#    deep mode so the read buffers and strings are exercised too.
if [ "$CHECK_LEAKS" = 1 ]; then
	command -v leaks >/dev/null || { echo "error: --leaks needs the macOS leaks tool" >&2; exit 1; }
	for name in folder packed-by-port; do
		root="$OUT/root-$name"
		(
			cd "$root"
			printf '/quit\n' | env HOME="$OUT/home" LC_RUST_GROUP_DEEP=1 leaks --atExit -- ./clonk "/config:$OUT/$name-leaks.config" > "$OUT/$name-leaks.log" 2>&1 || true
		)
		summary=$(grep -aE '^Process [0-9]+: [0-9]+ leaks? for' "$OUT/$name-leaks.log" | head -1 || true)
		[ -n "$summary" ] || fail "$name: leaks produced no summary (see $OUT/$name-leaks.log)"
		if grep -aqE 'lc_group_|RustGroupBridge|clonk_resources' "$OUT/$name-leaks.log"; then
			fail "$name: a leak reaches the group bridge (see $OUT/$name-leaks.log)"
		fi
		echo "leaks $name: ${summary#Process *: } (none through the bridge)"
	done
fi

echo
if [ "$failures" -eq 0 ]; then
	echo "group differential: all fixtures behaved as pinned (logs in $OUT)"
else
	echo "group differential: $failures failure(s) (logs in $OUT)" >&2
	exit 1
fi
