#!/usr/bin/env bash
# Regenerate the C++ golden oracle for the differential parity harness.
#
# Produces parity/golden/parity_golden.json from the REAL engine determinism
# primitives (src/Fixed.h, src/Fixed.cpp SineTable, src/C4Random.h), the
# production script-host helper (src/C4ScriptKiller.h), coarse landscape
# traversal (src/C4LandscapePath.h), action-direction decisions
# (src/C4ActionDirection.h), and active solid-mask bitmap sampling
# (src/C4SolidMaskBitmap.h), complete landscape BlastFree methods, and the
# bottom-flight C4Object::ContactAction arm. The Rust side
# (rust/crates/lc-engine/src/parity_differential.rs) diffs against the committed
# JSON, so this script only needs to run when the C++ primitives or oracle
# coverage change.
#
# Usage: parity/oracle/gen_golden.sh   (run from anywhere)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
src="$repo/src"
out="$repo/parity/golden/parity_golden.json"
gen="$here/.gen" # working dir for generated build inputs
mkdir -p "$gen"

# 1. Strip src/Fixed.h into a standalone header: drop the StdCompiler/StdAdaptors
#    includes and the serialization CompileFunc; the C4Fixed math is unchanged.
awk '
  /^#include "StdCompiler.h"$/ { next }
  /^#include "StdAdaptors.h"$/ { next }
  /friend inline void CompileFunc/ { next }
  /^\/\/ CompileFunc for C4Fixed$/ { skip = 1 }
  skip && /^}/ { skip = 0; next }
  skip { next }
  { print }
' "$src/Fixed.h" > "$gen/oracle_fixed.h"

# 2. Lift the real SineTable array out of src/Fixed.cpp.
awk '
  /^long SineTable\[9001\] =/ { p = 1 }
  p { print }
  p && /};/ { exit }
' "$src/Fixed.cpp" > "$gen/sine_table.cpp"

# 3. Mechanically lift complete production method bodies. The standalone
#    oracle supplies only their surrounding state scaffolding; branch/loop/RNG
#    order executes byte-for-byte from src/ rather than from a transcription.
awk '
  /^void C4Game::ShakeObjects\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Game.cpp" > "$gen/shake_objects.inc"

awk '
  /^void C4Object::Fling\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_fling.inc"

# The first DFA_FLIGHT arm inside ContactAction is its bottom-contact path.
# Keep the whole arm (through, but not including, DFA_SCALE) so the decisive
# `(OCF_HitSpeed4 || fDisabled)` gate executes directly from production text.
awk '
  /^void C4Object::ContactAction\(\)/ { in_contact_action = 1 }
  in_contact_action && /^[[:space:]]*case DFA_FLIGHT:/ && !p { p = 1 }
  p && /^[[:space:]]*case DFA_SCALE:/ { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/contact_action_bottom_flight.inc"

for helper_spec in "Walk walk" "Kneel kneel" "Flat flat"; do
  set -- $helper_spec
  helper="$1"
  helper_lower="$2"
  awk -v helper="$helper" '
    $0 ~ "^bool ObjectAction" helper "\\(" { p = 1 }
    p { print }
    p && /^}$/ { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$src/C4ObjectCom.cpp" > "$gen/object_action_${helper_lower}.inc"
done

awk '
  /^bool C4Landscape::ClearPix\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/landscape_clear_pix.inc"

awk '
  /^int32_t C4Landscape::BlastFreePix\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/landscape_blast_free_pix.inc"

awk '
  /^void C4Landscape::BlastFree\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/landscape_blast_free.inc"

# 4. Compile the oracle against the real C4Random.h (no DEBUGREC), the real
#    C4ScriptKiller.h/C4LandscapePath.h/C4ActionDirection.h/
#    C4SolidMaskBitmap.h production helpers, and the generated header/table;
#    then run it to produce the golden JSON.
cxx="${CXX:-clang++}"
"$cxx" -std=c++20 -O0 \
  -I"$gen" -I"$src" \
  "$here/oracle_main.cpp" "$gen/sine_table.cpp" \
  -o "$gen/oracle"

"$gen/oracle" > "$out"
echo "wrote $out ($(wc -c < "$out") bytes)"
