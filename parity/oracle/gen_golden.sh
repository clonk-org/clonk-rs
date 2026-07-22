#!/usr/bin/env bash
# Regenerate the C++ golden oracle for the differential parity harness.
#
# Produces parity/golden/parity_golden.json from the REAL engine determinism
# primitives (src/Fixed.h, src/Fixed.cpp SineTable, src/C4Random.h), the
# production script-host helper (src/C4ScriptKiller.h), coarse landscape
# traversal (src/C4LandscapePath.h), action-direction decisions
# (src/C4ActionDirection.h), and active solid-mask bitmap sampling
# (src/C4SolidMaskBitmap.h), complete C4Object::DigOutMaterialCast and
# landscape BlastFree methods, and the bottom/top/side-flight
# C4Object::ContactAction arms. The Rust side
# (rust/crates/lc-engine/src/parity_differential.rs) diffs against the committed
# JSON, so this script only needs to run when the C++ primitives or oracle
# coverage change.
#
# Usage: parity/oracle/gen_golden.sh   (run from anywhere)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
oracle_repo="${LEGACYCLONK_ORACLE_ROOT:-$repo/../../vendor/legacyclonk-oracle}"
oracle_revision="${LEGACYCLONK_ORACLE_REVISION:-oracle-src-pinned}"
out="$repo/parity/golden/parity_golden.json"
gen="$here/.gen" # working dir for generated build inputs
if ! oracle_commit="$(git -C "$oracle_repo" rev-parse --verify "$oracle_revision^{commit}")"; then
  echo "C++ oracle revision $oracle_revision not found in $oracle_repo" >&2
  exit 1
fi
mkdir -p "$gen"
oracle_snapshot="$gen/oracle-src-$oracle_commit"
if [[ ! -f "$oracle_snapshot/.complete" ]]; then
  mkdir -p "$oracle_snapshot"
  git -C "$oracle_repo" archive "$oracle_commit" src | tar -x -C "$oracle_snapshot"
  touch "$oracle_snapshot/.complete"
fi
src="$oracle_snapshot/src"

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
  /^void C4Landscape::ExecuteScan\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/landscape_execute_scan.inc"

awk '
  /^int32_t C4Landscape::DoScan\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/landscape_do_scan.inc"

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

# Compile the complete missing/incomplete-target check and its decisive
# callback -> AssignRemoval -> return block verbatim. Starting at the unique
# production comment makes the extraction fail if that exact section moves.
awk '
  /^[[:space:]]*case DFA_CONNECT:/ { in_connect = 1 }
  in_connect && /^[[:space:]]*\/\/ Line destruction check:/ && !p { p = 1 }
  p { print }
  p && /^[[:space:]]*}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_connect_missing_target.inc"

# The later fBroke arm is the geometry/LineConnect failure path. Its callback
# intentionally has no arguments, unlike the missing-target branch above.
awk '
  /^[[:space:]]*case DFA_CONNECT:/ { in_connect = 1 }
  in_connect && /^[[:space:]]*\/\/ Line fBroke$/ && !p { p = 1 }
  p { print }
  p && /^[[:space:]]*}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_connect_geometry_break.inc"

# C4Shape::LineConnect's first guard is enough to force the geometry branch
# without scaffolding the landscape-dependent path/bend search that follows.
awk '
  /^bool C4Shape::LineConnect\(/ { in_line_connect = 1 }
  in_line_connect && /^[[:space:]]*if \(VtxNum < 2\) return false;$/ {
    print
    found = 1
    exit
  }
  END { if (!found) exit 1 }
' "$src/C4Shape.cpp" > "$gen/shape_line_connect_vertex_guard.inc"

awk '
  /^void C4Object::DigOutMaterialCast\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_dig_out_material_cast.inc"

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

# The ceiling and wall DFA_FLIGHT arms contain the independent
# `(OCF_HitSpeed3 || fDisabled)` gates. Extract each complete arm, plus the
# shared unresolved-flight tail that runs after a tumble's switch `break`.
awk '
  /^void C4Object::ContactAction\(\)/ { in_contact_action = 1 }
  in_contact_action && /^[[:space:]]*\/\/ Hit Ceiling/ { in_section = 1 }
  in_section && /^[[:space:]]*case DFA_FLIGHT:/ && !p { p = 1 }
  p && /^[[:space:]]*case DFA_DIG:/ { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/contact_action_top_flight.inc"

awk '
  /^void C4Object::ContactAction\(\)/ { in_contact_action = 1 }
  in_contact_action && /^[[:space:]]*\/\/ Hit Left Wall/ { in_section = 1 }
  in_section && /^[[:space:]]*case DFA_FLIGHT:/ && !p { p = 1 }
  p && /^[[:space:]]*case DFA_WALK:/ { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/contact_action_left_flight.inc"

awk '
  /^void C4Object::ContactAction\(\)/ { in_contact_action = 1 }
  in_contact_action && /^[[:space:]]*\/\/ Hit Right Wall/ { in_section = 1 }
  in_section && /^[[:space:]]*case DFA_FLIGHT:/ && !p { p = 1 }
  p && /^[[:space:]]*case DFA_WALK:/ { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/contact_action_right_flight.inc"

awk '
  /^void C4Object::ContactAction\(\)/ { in_contact_action = 1 }
  in_contact_action && /^[[:space:]]*\/\/ Flight stuck/ { p = 1 }
  p && /^}$/ { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/contact_action_flight_stuck.inc"

for helper_spec in "Walk walk" "Kneel kneel" "Flat flat" "Tumble tumble" "Scale scale" "Hangle hangle"; do
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

# C4Rect::Scaled is the production truncation used to map a game-unit rect into
# a scaled definition's bitmap space. Lift the whole method.
awk '
  /^C4Rect C4Rect::Scaled\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Rect.cpp" > "$gen/rect_scaled.inc"

# The post-load percent->float conversion that feeds every scaled draw call.
# Anchored inside C4Def::Load so the extraction fails if the statement moves.
awk '
  /^bool C4Def::Load\(/ { p = 1 }
  p && /^[[:space:]]*Scale = C4DefCore::Scale \/ 100\.0f;$/ { print; found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Def.cpp" > "$gen/def_scale_from_defcore.inc"

# C4Def::Picture2Facet's decisive statement: the phase offset is composed in
# GAME units and only the resulting rect is scaled into bitmap space, so the
# truncation applies to the already-offset x. Anchored on the signature.
awk '
  /^void C4Def::Picture2Facet\(/ { p = 1; next }
  p && /const auto scaledRect =/ { print; found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Def.cpp" > "$gen/def_picture2facet_rect.inc"

# 4. Compile the oracle against the real C4Random.h (no DEBUGREC), the real
#    C4ScriptKiller.h/C4LandscapePath.h/C4ActionDirection.h/
#    C4SolidMaskBitmap.h production helpers, and the generated header/table;
#    then run it to produce the golden JSON. The pinned ExecuteScan body keeps
#    its intentional nested-if formatting, so suppress only that style warning.
cxx="${CXX:-clang++}"
"$cxx" -std=c++20 -O0 \
  -Wno-dangling-else \
  -I"$gen" -I"$src" \
  "$here/oracle_main.cpp" "$gen/sine_table.cpp" \
  -o "$gen/oracle"

"$gen/oracle" > "$out"
echo "wrote $out ($(wc -c < "$out") bytes) from $oracle_revision ($oracle_commit)"
