#!/usr/bin/env bash
# Regenerate the C++ golden oracle for the differential parity harness.
#
# Produces parity/golden/parity_golden.json from the REAL engine determinism
# primitives (src/Fixed.h, src/Fixed.cpp SineTable, src/C4Random.h), the
# production script-host helper (src/C4ScriptKiller.h), coarse landscape
# traversal (src/C4LandscapePath.h), FnEval/DirectExec context selection
# (src/C4Script.cpp, src/C4AulExec.cpp), action-direction decisions
# (src/C4ActionDirection.h), mechanically extracted DFA_PUSH/PULL/FIGHT
# direction blocks and active solid-mask bitmap sampling
# (src/C4SolidMaskBitmap.h), the mechanically extracted container
# CopyMotion/GetSpeed/ObjectComCancelAttach helpers, complete
# C4Object::DigOutMaterialCast and landscape BlastFree methods, and the
# bottom/top/side-flight C4Object::ContactAction
# arms, plus C4PlayerList::GetCount and Join's player-capacity gate, and the
# complete callback-bearing C4Effect lifecycle. The Rust side
# (crates/clonk-engine/src/parity_differential.rs) diffs against the committed
# JSON, so this script only needs to run when the C++ primitives or oracle
# coverage change.
#
# Usage: parity/oracle/gen_golden.sh   (run from anywhere)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
# The pinned C++ tree remains reachable in this repository's history even
# though it is no longer present in the working tree. An external oracle
# checkout may be selected for differential development.
default_oracle_revision="7d43b47b7d789b533f32d005e64596e0a07019cd"
oracle_repo="${LEGACYCLONK_ORACLE_ROOT:-$repo}"
oracle_revision="${LEGACYCLONK_ORACLE_REVISION:-$default_oracle_revision}"
out="$repo/parity/golden/parity_golden.json"
gen="$here/.gen" # working dir for generated build inputs
if ! oracle_commit="$(git -C "$oracle_repo" rev-parse --verify "$oracle_revision^{commit}")"; then
  {
    echo "C++ oracle revision $oracle_revision not found in $oracle_repo"
    echo "The default revision is stored in this repository's full Git history."
    echo "For a shallow clone, fetch the missing history; for an external checkout,"
    echo "set LEGACYCLONK_ORACLE_ROOT and optionally LEGACYCLONK_ORACLE_REVISION."
  } >&2
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

# FnEval's complete production body selects DirectExec's receiver from the
# active C4Aul context: object definition, definition, then Game.Script.
awk '
  /^static C4Value FnEval\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Script.cpp" > "$gen/script_fn_eval.inc"

# DirectExec's temporary child gets an object's current Def and LocalNamed
# table, then registers under the selected receiver. Lift that decisive setup
# block verbatim; the focused oracle supplies only the surrounding objects.
awk '
  /^C4Value C4AulScript::DirectExec\(/ { in_direct_exec = 1 }
  in_direct_exec && /^[[:space:]]*if \(pObj\)$/ { p = 1 }
  p { print }
  p && /^[[:space:]]*pScript->Reg2List\(Engine, this\);$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4AulExec.cpp" > "$gen/script_direct_exec_scope.inc"

# A definition-commanded effect resolves its callback code through
# idCommandTarget but C4Effect::Execute still passes pCommandTarget as the
# callback receiver. Lift the complete Execute body, the real script-function
# engine-call forwarding and context setup, and the two position hosts whose
# implicit target is cthr->Obj. The focused scaffold makes the affected
# pForObj/carrier distinct from that nullable callback receiver.
awk '
  /^void C4Effect::Execute\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Effect.cpp" > "$gen/effect_execute.inc"

awk '
  /^C4Value C4Effect::DoCall\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Effect.cpp" > "$gen/effect_do_call.inc"

awk '
  /^C4Value C4AulScriptFunc::Exec\(C4Object \*pObj,/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4AulExec.cpp" > "$gen/aul_script_func_exec.inc"

# Keep the strictness predicate and the conversion helper in the C++ fixture
# as source extracts too. The effect-callback case below relies on this exact
# onlyWarn plus non-strict decision: pre-STRICT3 callbacks warn and run, while
# STRICT3 callbacks reject before their body can observe or alias the
# incompatible object argument.
awk '
  /^bool C4AulScriptFunc::HasStrictNil\(\) const noexcept/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4AulExec.cpp" > "$gen/aul_script_func_has_strict_nil.inc"

awk '
  /^static void ErrorOrWarning\(/ { p = 1 }
  p { print }
  p && /^}$/ { closures++ }
  p && closures == 3 { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4AulExec.cpp" > "$gen/aul_parameter_conversion.inc"

awk '
  /^C4Value C4AulExec::Exec\(C4AulScriptFunc \*pSFunc,/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4AulExec.cpp" > "$gen/aul_exec_script_context.inc"

# FnSqrt corrects a truncated double root with two `iSqrt * iSqrt`
# comparisons. Those products are C4ValueInt, so the second one wraps above
# 46340^2 and the correcting decrement never runs. Lift the body verbatim
# rather than restating it: the overflow is what the port has to reproduce.
awk '
  /^static C4ValueInt FnSqrt\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Script.cpp" > "$gen/script_fn_sqrt.inc"

for host_name in GetX GetY; do
  awk -v host_name="$host_name" '
    $0 ~ "^static std::optional<C4ValueInt> Fn" host_name "\\(" { p = 1 }
    p { print }
    p && /^}$/ { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$src/C4Script.cpp" > "$gen/script_fn_${host_name}.inc"
done

# Network game startup must place the synchronized C4GameParameters lists,
# not a client's local Scenario.txt lists. Lift the complete production
# conversion and placement methods so the HarpoonRace fixture below executes
# the pinned source text rather than an independently rewritten algorithm.
awk '
  /^void C4SGame::ConvertGoals\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Scenario.cpp" > "$gen/scenario_convert_goals.inc"

awk '
  /^void C4SGame::ClearOldGoals\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Scenario.cpp" > "$gen/scenario_clear_old_goals.inc"

awk '
  /^void C4Game::InitRules\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Game.cpp" > "$gen/game_init_rules.inc"

awk '
  /^void C4Game::InitGoals\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Game.cpp" > "$gen/game_init_goals.inc"

# Player admission counts every linked C4Player, then rejects before duplicate
# file checks, allocation, or initialization when count+1 exceeds MaxPlayers.
# Bound both extracts to their production functions and the Join extract to
# the next named section so source movement cannot silently select another
# condition with similar text.
awk '
  /^int C4PlayerList::GetCount\(\) const$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PlayerList.cpp" > "$gen/player_list_get_count.inc"

awk '
  /^C4Player \*C4PlayerList::Join\(/ { in_join = 1 }
  in_join && /^}$/ { exit 1 }
  in_join && /^[[:space:]]*\/\/ Too many players$/ { p = 1 }
  p && /if \(GetCount\(\) \+ 1 > Game.Parameters.MaxPlayers\)/ { conditions++ }
  p && /Log\(C4ResStrTableKey::IDS_PRC_TOOMANYPLRS, Game.Parameters.MaxPlayers\);/ { logs++ }
  p && /^[[:space:]]*return nullptr;$/ { returns++ }
  p && /^[[:space:]]*\/\/ Check duplicate file usage$/ { bounded = 1; exit }
  p { print }
  END { if (!bounded || conditions != 1 || logs != 1 || returns != 1) exit 1 }
' "$src/C4PlayerList.cpp" > "$gen/player_join_capacity.inc"

# The extracted methods traverse and mutate C4IDList. Lift the small list
# operations they call as well, including both production findId overloads.
awk '
  /^static auto findId\(/ { p = 1 }
  p { print }
  p && /^}$/ {
    closures++
    if (closures == 2) { found = 1; exit }
  }
  END { if (!found) exit 1 }
' "$src/C4IDList.cpp" > "$gen/id_list_find.inc"

for method_spec in \
  "Clear id_list_clear" \
  "GetID id_list_get_id" \
  "GetIDCount id_list_get_id_count" \
  "SetIDCount id_list_set_id_count" \
  "GetNumberOfIDs id_list_get_number_of_ids"
do
  set -- $method_spec
  method="$1"
  output="$2"
  awk -v method="$method" '
    $0 ~ "^[A-Za-z0-9_:<> ]+C4IDList::" method "\\(" { p = 1 }
    p { print }
    p && /^}$/ { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$src/C4IDList.cpp" > "$gen/${output}.inc"
done

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

# Lift the exact direction-decision blocks from DFA_PUSH, DFA_PULL, and
# DFA_FIGHT. The focused scaffold supplies only their already-computed raw
# xdir/target positions and records SetDir calls; sign/position tests and
# their independent-if ordering remain production C4Object.cpp text.
awk '
  /^void C4Object::ExecAction\(\)/ { in_exec = 1 }
  in_exec && /^}$/ { exit 1 }
  in_exec && /^[[:space:]]*case DFA_/ {
    if (in_push && $0 !~ /case DFA_PUSH:/) exit 1
    if ($0 ~ /case DFA_PUSH:/) in_push = 1
  }
  in_push && /^[[:space:]]*\/\/ Phase by XDir$/ { p = 1 }
  p { print }
  p && /if \(xdir > 0\).*SetDir\(DIR_Right\)/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_push_direction.inc"

awk '
  /^void C4Object::ExecAction\(\)/ { in_exec = 1 }
  in_exec && /^}$/ { exit 1 }
  in_exec && /^[[:space:]]*case DFA_/ {
    if (in_pull && $0 !~ /case DFA_PULL:/) exit 1
    if ($0 ~ /case DFA_PULL:/) in_pull = 1
  }
  in_pull && /^[[:space:]]*\/\/ Phase by XDir$/ { p = 1 }
  p { print }
  p && /if \(xdir > 0\).*SetDir\(DIR_Right\)/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_pull_direction.inc"

awk '
  /^void C4Object::ExecAction\(\)/ { in_exec = 1 }
  in_exec && /^}$/ { exit 1 }
  in_exec && /^[[:space:]]*case DFA_/ {
    if (in_fight && $0 !~ /case DFA_FIGHT:/) exit 1
    if ($0 ~ /case DFA_FIGHT:/) in_fight = 1
  }
  in_fight && /^[[:space:]]*\/\/ Direction$/ { p = 1 }
  p { print }
  p && /if \(Action.Target->x < x\) SetDir\(DIR_Left\)/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_fight_direction.inc"

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

# C4PlayerInfoList::FindSavegameResumePlayerInfo's per-level predicate: the four
# MatchingLevel passes RestoreSavegameInfos runs over the unassociated players
# (C4PlayerInfo.cpp:1102-1118, driven from :1373-1391). Bound to the switch and
# to all four case labels, so a change to the surrounding client traversal, or a
# dropped level, fails the extraction instead of silently narrowing the oracle.
# PML_PlrFileName's deliberate fallthrough into PML_PlrName is part of the
# extracted text.
awk '
  /^C4PlayerInfo \*C4PlayerInfoList::FindSavegameResumePlayerInfo\(/ { in_fn = 1 }
  in_fn && /switch \(iMatchLvl\)/ { p = 1 }
  p { print }
  p && /case PML_PlrFileName:/ { file_name++ }
  p && /case PML_PlrName:/ { plr_name++ }
  p && /case PML_PrefColor:/ { pref_color++ }
  p && /case PML_Any:/ { any++; closing = 1; next }
  p && closing && /^[[:space:]]*}$/ { found = 1; exit }
  END {
    if (!found || file_name != 1 || plr_name != 1 || pref_color != 1 || any != 1) exit 1
  }
' "$src/C4PlayerInfo.cpp" > "$gen/savegame_matching_switch.inc"

# The predicate compares through the production string and path helpers, so lift
# those rather than restating their semantics: CharCapital carries the three
# Latin-1 umlaut foldings, and GetFilename's separator set is what decides where
# a player file's basename starts.
awk '
  /^char CharCapital\(char cChar\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Strings.cpp" > "$gen/char_capital.inc"

awk '
  /^bool SEqualNoCase\(const char \*szStr1, const char \*szStr2, size_t iLen\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Strings.cpp" > "$gen/sequal_no_case.inc"

awk '
  /^char \*GetFilename\(char \*szPath\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/StdFile.cpp" > "$gen/get_filename.inc"

# 3z. Lift C4PXSSystem's slot allocator and its counterpart free. Allocation
#     order is the whole determinism story for PXS: `New` returns the first
#     `Mat == MNone` slot of the first chunk with space, so a freed slot is
#     reused at its old index and the execution order that follows is fixed by
#     it. Both bodies touch only `Chunk`/`iChunkPXS`, so the oracle needs no
#     landscape or material scaffolding to run them.
awk '
  /^C4PXS \*C4PXSSystem::New\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PXS.cpp" > "$gen/pxs_new.inc"

awk '
  /^void C4PXSSystem::Delete\(C4PXS \*pPXS\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PXS.cpp" > "$gen/pxs_delete.inc"

# Execute and Synchronize are kept as distinct extracts rather than folded into
# the Create/Cast/Load loop below. The integrated lifecycle fixture includes
# both at once: Execute walks live slots in stable chunk/slot order and frees
# each chunk only if it is empty when its outer-loop turn begins
# (src/C4PXS.cpp:218-240), while Synchronize resets only the public execution
# count (src/C4PXS.cpp:401-404).
awk '
  /^void C4PXSSystem::Execute\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PXS.cpp" > "$gen/pxs_system_execute.inc"

awk '
  /^void C4PXSSystem::Synchronize\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PXS.cpp" > "$gen/pxs_synchronize.inc"

# Save is extracted separately because the lifecycle fixture executes the real
# whole-component serialization after every transition. Its two-pass policy is
# the parity point: no live chunk deletes the component, but if any live chunk
# exists every allocated chunk is written byte-for-byte in slot order
# (src/C4PXS.cpp:324-360).
awk '
  /^bool C4PXSSystem::Save\(C4Group &hGroup\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PXS.cpp" > "$gen/pxs_save.inc"

# 3y. Lift C4MaterialMap::mrfPoof. Its mass-move and PXS-position arms consume
#     the synchronised RNG twice through Rnd3 and gate a smoke puff and a sound
#     on those draws, so the *order and count* of the draws is parity state, not
#     presentation. Everything the arms touch besides Rnd3 is a side effect the
#     oracle records rather than performs.
awk '
  /^bool C4MaterialMap::mrfPoof\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/mrf_poof.inc"

# 3x. Lift C4MassMoverSet::Create's slot scan. Where a mover lands decides
#     whether the descending Execute pass reaches it again this frame or next,
#     so the cyclic search — start after CreatePtr, wrap at the chunk end, first
#     `Mat == MNone` wins, CreatePtr follows the slot taken — is parity state.
#     Two things are dropped, both instrumentation rather than behaviour: the
#     `LC_RNG_TRACE` block this fork adds for tracing, and the DEBUGREC record
#     (already inactive, since the oracle compiles without DEBUGREC).
awk '
  /^bool C4MassMoverSet::Create\(/ { p = 1 }
  p && /getenv\("LC_RNG_TRACE"\)/ { trace = 1; next }
  trace && /^\t}$/ { trace = 0; next }
  trace { next }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MassMover.cpp" > "$gen/mass_mover_create.inc"

# 3w. Lift Splash. It is the only liquid-entry effect that draws from the
#     synchronised stream, and its draw COUNT is landscape-dependent: the
#     extraction inside the loop empties the pixel it is drawing from, so the
#     second iteration takes two draws where the first took four. The two
#     "force argument evaluation order" pairs are the reason the body is lifted
#     rather than restated — the r2-before-r1 order is the parity fact.
awk '
  /^void Splash\(int32_t tx, int32_t ty, int32_t amt, C4Object \*pByObj\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Effect.cpp" > "$gen/splash.inc"

# 3v. Lift C4Object::UpdateInLiquid and IsInLiquidCheck. The probe is taken at
#     `y + Def->Float * Con / FullCon - 1`, so a growing or shrinking object
#     starts swimming at a different pixel; entry is edge-triggered, and the
#     splash it fires on that edge is the draw that has to land on the same
#     frame in both engines. C4Movement's copy of this block additionally clears
#     `fNoAttach`, which is why the movement path is compared separately.
awk '
  /^void C4Object::UpdateInLiquid\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/update_in_liquid.inc"

awk '
  /^bool C4Object::IsInLiquidCheck\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/is_in_liquid_check.inc"

# 3u. Lift C4Game::BlastObjects and C4Object::Blast. The selection chain
#     decides who is
#     hit at all — direct hit widens the shape by five pixels on every side,
#     while the shock wave gates on category, NoHorizontalMove, Grab, and a
#     DFA_FLOAT action — and the fling force is another forced-evaluation-order
#     pair whose `p1` alone consumes an Rnd3 draw. Both are lifted rather than
#     restated because the order of those two lines is the parity fact.
awk '
  /^void C4Game::BlastObjects\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Game.cpp" > "$gen/blast_objects.inc"

awk '
  /^void C4Object::Blast\(int32_t iLevel, int32_t iCausedBy\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_blast.inc"

# 3t. Lift C4Weather::Execute and the C4SVal::Evaluate the wind target reads
#     through. The disaster block is four gates in a fixed order, and each gate
#     draws its `Random(100)` test EVEN WHEN the level is zero — so how many
#     draws a tick takes depends on which outer gates hit, not on which
#     disasters happen. Three of the four then use the same forced
#     r2-before-r1 evaluation order seen elsewhere.
awk '
  /^void C4Weather::Execute\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Weather.cpp" > "$gen/weather_execute.inc"

awk '
  /^int32_t C4SVal::Evaluate\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Scenario.cpp" > "$gen/c4sval_evaluate.inc"

# 3s. Lift C4Shape::ContactCheck, the per-pixel probe every step of
#     C4Object::DoMovement runs. It decides ContactCNAT, ContactCount and the
#     per-vertex VtxContactCNAT/VtxContactMat, so a vertex that answers
#     differently by one pixel moves the object differently for the rest of the
#     frame. Its density reads go through GetPix's border rules, where a CLOSED
#     border answers MCVehic — solid — rather than sky.
awk '
  /^void C4Object::TargetBounds\(int32_t &ctco, int32_t limit_low, int32_t limit_hi, int32_t cnat_low, int32_t cnat_hi\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/target_bounds.inc"

awk '
  /^bool C4Shape::Attach\(int32_t &cx, int32_t &cy, uint8_t cnat_pos\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Shape.cpp" > "$gen/shape_attach.inc"

awk '
  /^bool C4Shape::ContactCheck\(int32_t cx, int32_t cy\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Shape.cpp" > "$gen/shape_contact_check.inc"

# C4Object::ContactCheck is the callback half of the same probe. Rename only
# its definition so the scaffold can wrap the production body and record the
# per-step result without rewriting Shape.ContactCheck inside it.
awk '
  /^int32_t C4Object::ContactCheck\(int32_t iAtX, int32_t iAtY\)$/ {
    p = 1
    sub(/C4Object::ContactCheck/, "C4Object::ContactCheckNative")
  }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/object_contact_check.inc"

# Rotation updates every live contact vertex before the second collision walk.
# Compile the real fixed-trigonometry body so the rotation matrix and fixtoi
# rounding cannot drift from C++.
awk '
  /^void C4Shape::Rotate\(int32_t iAngle, bool bUpdateVertices\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Shape.cpp" > "$gen/shape_rotate.inc"

# 3q2. Lift C4PXS::Execute in full. `pxs_allocation` pins only the allocator;
#      this is the per-tick step itself — raw C4Fixed position/velocity, the
#      gravity accumulation, the airborne wind branch and its exact pair of
#      Random(1200) draws, and the _PathFree fast path. PXS is on the bit-exact
#      list, and comparing only fixtoi() here would mask a sub-pixel desync.
awk '
  /^void C4PXS::Execute\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PXS.cpp" > "$gen/pxs_execute.inc"

# 3q3. Lift mrfInsertCheck and the FindMatSlide it calls. This is the arm every
#      falling pixel takes on landing, and its RNG ledger is property-dependent:
#      a rough contact spends two draws on the splash roll, an incendiary
#      material another two on its smoke, and a found slide one more. It also
#      rewrites the pixel's position and velocity, so a wrong branch moves the
#      pixel and desynchronises the stream at once.
awk '
  /^bool mrfInsertCheck\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/mrf_insert_check.inc"

awk '
  /^bool C4Landscape::FindMatSlide\(int32_t &fx, int32_t &fy, int32_t ydir, int32_t mdens, int32_t mslide\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/find_mat_slide.inc"

# 3q3d. Lift C4Landscape::InsertMaterial and the two path helpers it walks.
#       This is the landscape-mutation half of a material reaction: where the
#       inserted pixel actually comes to rest, and whether it lands as a
#       landscape pixel or as a PXS.
#
#       Its decisions are all order-sensitive and several read as typos until
#       you check them against the source. A density-0 material returns **true**
#       having done nothing. The bounds test accepts `ty == Height` but stops
#       `tx` at `Width - 1`. And the non-push-pull climb applies its primitive
#       slide as two INDEPENDENT `if`s, so a pixel with both neighbours free
#       steps left and then right and ends where it started -- a port that wrote
#       `else if` would drift one pixel per iteration.
awk '
  /^bool C4Landscape::InsertMaterial\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/insert_material.inc"

awk '
  /^bool C4Landscape::FindMatPath\(int32_t &fx, int32_t &fy, int32_t ydir, int32_t mdens, int32_t mslide\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/find_mat_path.inc"

awk '
  /^bool C4Landscape::FindMatPathPush\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/find_mat_path_push.inc"

# 3q3e. Lift C4Landscape::ExtractMaterial and the FindMatTop walk it depends on.
#       ExtractMaterial does NOT clear the pixel it was handed: it identifies the
#       material there, walks FindMatTop up that material's own column, and
#       clears the pixel it ends on. Extracting from the middle of a column
#       therefore removes its TOP, which is why a port that simply cleared the
#       requested pixel would leave a hole in the wrong place.
#
#       FindMatTop's loop is also indented in a way that misreads. `if (fLeft)`
#       carries no braces, so it governs only the left if/else-if chain; the
#       following `if (fRight)` is an INDEPENDENT statement despite sitting one
#       level in. Left and right are therefore both examined in the same cslide
#       iteration, and the `break` leaves cslide at the value that matched --
#       which is the distance the slide then applies.
awk '
  /^void C4Landscape::FindMatTop\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/find_mat_top.inc"

awk '
  /^int32_t C4Landscape::ExtractMaterial\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/extract_material.inc"

# 3q3f. Lift C4Landscape::DigFree, DigFreeMat and DigFreePix.
#       DigFree walks a circle row by row, and two of its details are easy to
#       "tidy" into something that digs a different shape:
#
#         * `iLineWidth` is declared OUTSIDE the row loop and the bottom-edge
#           pass reads it after the loop has ended -- so the bottom edge is as
#           wide as the LAST row was, not as wide as the circle. A port that
#           recomputed it there would clear a different set of pixels.
#         * a row whose half-width computes to 0 still digs one pixel, via the
#           `+ (iLineWidth == 0)` bump that appears in the loop bound and again
#           in the right-hand edge position.
#
#       DigFreePix itself probes instability for EVERY pixel it is handed,
#       including sky and including material whose DigFree is 0 -- the probe sits
#       after the conditional clear, not inside it.
awk '
  /^int32_t C4Landscape::DigFreePix\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/dig_free_pix.inc"

awk '
  /^void C4Landscape::DigFree\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/dig_free.inc"

#       DigFreeMat is a separate exact-material rectangle walk. Its outer
#       MatValid guard, x-major/y-minor order and GetMat prefilter are all part
#       of the production body rather than reimplemented in the oracle.
awk '
  /^void C4Landscape::DigFreeMat\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Landscape.cpp" > "$gen/dig_free_mat.inc"

# 3q3g. Lift the builtin reaction-selection loop out of CrossMapMaterials.
#       This is the decision every arm section above depends on: which builtin
#       reaction a (PXS material, landscape material) pair gets in the first
#       place. Only the first loop is lifted -- the rest of CrossMapMaterials
#       initialises textures and PXS facets and would drag in the whole graphics
#       layer -- following the same block-lift approach as
#       script_direct_exec_scope.inc.
#
#       Two things make it worth pinning. The chain is an if/else-if LADDER, so
#       its order is the behaviour: InMatConvert wins over everything, then
#       poof, then incinerate, then corrode, then insert. And every branch but
#       the convert one sits behind `MatDensity(PXS) <= MatDensity(LS)`, so a
#       heavier PXS hitting lighter landscape gets NO reaction at all -- a gate
#       that is easy to lose when the ladder is rewritten as a match.
awk '
  /^void C4MaterialMap::CrossMapMaterials\(\)/ { p = 1 }
  p { print }
  p && /^[[:space:]]*SetMatReaction\(iMatPXS, iMatLS, pReaction\);$/ { tail = 1 }
  tail && /^[[:space:]]*}$/ { closes++ }
  tail && closes == 2 { print "}"; found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/cross_map_reactions.inc"

# 3q3h. Lift the custom-reaction overlay and SetMatReaction from
#       C4MaterialMap::CrossMapMaterials. The overlay resolves ConvertMat,
#       expands literal/category TargetSpec values (including inverse specs),
#       and applies reactions in authored order. SetMatReaction supplies the
#       Reverse swap and the landscape-major flat-table layout.
awk '
  /^[[:space:]]*\/\/ evaluate reactions for that material$/ { p = 1 }
  p {
    print
    line = $0
    opens += gsub(/{/, "{", line)
    closes += gsub(/}/, "}", line)
    if (opens && opens == closes) { found = 1; exit }
  }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/custom_reaction_overlay.inc"

awk '
  /^void C4MaterialMap::SetMatReaction\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/set_mat_reaction.inc"

awk '
  /^bool mrfUserCheck\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/material_user_check.inc"

# 3q3i. Lift the unattached half of C4Object::DoMovement and the force helpers
#       it drives. This is the per-pixel collision loop: the fixed-point target
#       is accumulated first, clamped by SideBounds/VerticalBounds, and then
#       walked ONE PIXEL AT A TIME with a ContactCheck at each step.
#
#       The two halves are deliberately asymmetric, and that is the behaviour:
#       a horizontal contact redirects xdir into ydir and rubs FRICTION off
#       ydir, while a vertical contact rubs friction off xdir FIRST and then
#       picks a response from the contact vertices' CNAT — slide left, else
#       slide right, else (only for a non-Alive rotatable object on a single
#       contact) bleed ydir into rdir, else zero ydir outright.
#
#       On contact the loop also rewrites the fixed coordinate back to the whole
#       pixel (`fix_x = itofix(x)`), discarding the sub-pixel remainder. A port
#       that kept the remainder would drift a subpixel per contact.
awk '
  /^const C4Fixed FRedirect = /  { print }
  /^const C4Fixed FFriction = /  { print; exit }
' "$src/C4Movement.cpp" > "$gen/movement_constants.inc"

awk '
  /^void RedirectForce\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/redirect_force.inc"

awk '
  /^void ApplyFriction\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/apply_friction.inc"

awk '
  /^bool ContactVtxCNAT\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/contact_vtx_cnat.inc"

awk '
  /^int32_t ContactVtxWeight\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/contact_vtx_weight.inc"

awk '
  /^int32_t ContactVtxFriction\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/contact_vtx_friction.inc"

awk '
  /^void C4Object::SideBounds\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/side_bounds.inc"

awk '
  /^void C4Object::VerticalBounds\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/vertical_bounds.inc"

#       The unattached block is lifted as a STATEMENT, not a function: it is the
#       `if (!Action.t_attach) { ... }` inside DoMovement, and the surrounding
#       declarations are supplied by the scaffold that includes it. Stopping at
#       the attached branch keeps the lift to the half this section covers.
awk '
  /^\tif \(!Action.t_attach\) \/\/ Unattached movement$/ { p = 1 }
  p && /^\tif \(Action.t_attach\)/ { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/do_movement_unattached.inc"

#       The attached branch is the other half of that same `if`, and is lifted
#       the same way. It is a different walk, not a variant of the unattached
#       one: every step re-runs Shape.Attach, an attachment that moves ctx/cty
#       overrides the momentum target and zeroes that axis' velocity, and a
#       contact aborts the walk by snapping the target back to the current
#       position. Stopping at the rotation comment keeps the lift to the
#       movement half.
awk '
  /^\tif \(Action.t_attach\) \/\/ Attached movement$/ { p = 1 }
  p && /^\t\/\/ Rotation$/ { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/do_movement_attached.inc"

# The immediately following rotation block is a second one-degree-at-a-time
# collision walk. Keep it mechanically lifted through the circle-bound tail;
# the scaffold supplies the DoMovement locals and an absolute UpdateShape.
awk '
  /^\t\/\/ Rotation$/ { p = 1 }
  p && /^\tif \(Number == / { found = 1; exit }
  p { print }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/do_movement_rotation.inc"

# 3q3b. Lift mrfIncinerate. It is the one reaction whose arms are asymmetric in
#       a way a port is likely to flatten: `meeMassMove` and `meePXSPos` try to
#       incinerate and report **unhandled** when they cannot, while `meePXSMove`
#       runs the insertion check FIRST -- a splash or slide that prevents the
#       interaction returns unhandled before anything burns -- and then, if the
#       pixel fails to ignite, inserts it rather than dropping it. Any event
#       outside those three never reaches `C4Landscape::Incinerate` at all,
#       because the switch has no default arm.
awk '
  /^bool C4MaterialMap::mrfIncinerate\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/mrf_incinerate.inc"

# 3q3c. Lift mrfCorrode. Its RNG ledger is the point: a NON-user reaction rolls
#       `Random(100) < Corrosive` and only then `Random(100) < Corrode`, and C++'s
#       `&&` short-circuits -- a failed first roll spends ONE draw, not two. A
#       user reaction spends one draw against its own CorrosionRate instead.
#       The effect gates are conditional in the same way: `!Random(5)` opens the
#       smoke, and `Random(3)` for its level is drawn ONLY when it does, before
#       `!Random(20)` decides the sound. Every one of those is a synchronised
#       draw, so a port that evaluated both rolls eagerly, or drew the smoke
#       level unconditionally, would desynchronise the stream while producing
#       the same landscape.
awk '
  /^bool C4MaterialMap::mrfCorrode\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/mrf_corrode.inc"

# 3q4. Lift mrfUserCheck and mrfConvert. Convert carries two rules that a port
#      can silently lose: C++'s `case meePXSMove:` falls **through** into
#      `meePXSPos` for user-defined reactions (Rust has no implicit
#      fallthrough), and a *successful* conversion still returns false — it is
#      "not handled", so the caller keeps going — while a conversion to an
#      unloaded or sky target returns true and kills the pixel.
awk '
  /^bool mrfUserCheck\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/mrf_user_check.inc"

awk '
  /^bool C4MaterialMap::mrfConvert\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/mrf_convert.inc"

# 3q5. Lift mrfInsert. Its splash/slide check is `!fUserDefined`-gated INSIDE
#      the movement case, because a user-defined reaction already ran the same
#      check through mrfUserCheck. Losing that gate runs the check twice and
#      doubles the synchronized draws on every inserting pixel.
awk '
  /^bool C4MaterialMap::mrfInsert\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Material.cpp" > "$gen/mrf_insert.inc"

# 3s. Lift `Create` and `Cast`, the two callers of the already-lifted `New`.
#     `Cast` draws its two randoms in an order the C++ had to force explicitly
#     with named locals, and the one drawn FIRST is the one used for ydir — so a
#     port reading them in argument order gets swapped velocities while drawing
#     exactly as many numbers.
#
#     `Load` comes along because its accept/reject matrix is entirely arithmetic
#     on the file length — a four-byte number-format tag is detected by the
#     remainder being exactly 4, not by a magic value — and because its
#     float-format conversion is applied ONLY to slots whose material is set,
#     leaving dead slots holding raw float bits.
for fn in Create Cast Load Clear; do
  awk -v fn="$fn" '
    $0 ~ ("^(C4PXS \\*|bool |void )C4PXSSystem::" fn "\\(") { p = 1 }
    p { print }
    p && /^}$/ { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$src/C4PXS.cpp" > "$gen/pxs_$(echo "$fn" | tr 'A-Z' 'a-z').inc"
done

awk '
  /^void C4PXS::Deactivate\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4PXS.cpp" > "$gen/pxs_deactivate.inc"

# 3r. Lift the container lifecycle: C4Object::Enter, Exit and Collect. These are
#     ordered state machines whose SHAPE is the parity fact — which script call
#     runs before which mutation, which rollback undoes a failed insert, and
#     which `Status` re-check aborts the rest after a callback removed one of
#     the two objects. A port that ran the same calls in a different order, or
#     skipped a re-check, would look right until a script used it.
awk '
  /^bool C4Object::ChangeDef\(C4ID idNew\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_change_def.inc"

awk '
  /^const C4Fixed HitSpeed1 =/ { p = 1 }
  p { print }
  p && /^const C4Fixed HitSpeed4 =/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/object_hit_speeds.inc"

awk '
  /^C4Fixed C4Object::GetSpeed\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_get_speed.inc"

awk '
  /^bool C4Object::Enter\(C4Object \*pTarget, bool fCalls, bool fCopyMotion, bool \*pfRejectCollect\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_enter.inc"

awk '
  /^void C4Object::CopyMotion\(C4Object \*from\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Movement.cpp" > "$gen/object_copy_motion.inc"

awk '
  /^bool ObjectComCancelAttach\(C4Object \*cObj\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectCom.cpp" > "$gen/object_com_cancel_attach.inc"

awk '
  /^bool C4Object::Exit\(int32_t iX, int32_t iY, int32_t iR, C4Fixed iXDir, C4Fixed iYDir, C4Fixed iRDir, bool fCalls\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_exit.inc"

awk '
  /^bool C4Object::Collect\(C4Object \*pObj\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_collect.inc"

# 3r2. Lift the linked-list operations that define container contents order.
#       `Add` owns both the category/id insertion rules and link allocation;
#       `Remove` and `ShiftContents` distinguish fresh-link reinsertion from a
#       rotation that preserves link identity. The other three bodies are the
#       complete pointer helpers those operations call.
awk '
  /^bool C4ObjectList::Add\(C4Object \*nObj, SortType eSort, C4ObjectList \*pLstSorted\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/object_list_add.inc"

awk '
  /^bool C4ObjectList::Remove\(C4Object \*pObj\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/object_list_remove.inc"

awk '
  /^C4ObjectLink \*C4ObjectList::GetLink\(C4Object \*pObj\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/object_list_get_link.inc"

awk '
  /^void C4ObjectList::RemoveLink\(C4ObjectLink \*pLnk\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/object_list_remove_link.inc"

awk '
  /^void C4ObjectList::InsertLink\(C4ObjectLink \*pLnk, C4ObjectLink \*pAfter\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/object_list_insert_link.inc"

awk '
  /^bool C4ObjectList::ShiftContents\(C4Object \*pNewFirst\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/object_list_shift_contents.inc"

# 3q. Lift C4Effect::Check, the negotiation every AddEffect runs before an
#     effect exists. It walks the whole list asking each live effect of at least
#     the new priority whether it objects; a Deny short-circuits, while an Annul
#     nominates that effect to absorb the new one — and the AnnulCalls form
#     additionally brackets the FxAdd call in temp-remove/temp-readd of every
#     effect above it. Which of those branches a port takes decides whether a
#     second effect exists at all.
# 3p. Lift C4Object::AssignRemoval, the object teardown. Every step is ordered
#     against a script callback that may already have deleted the object, and
#     the contents are torn down BEFORE the object leaves its own container —
#     a sequence that decides which callbacks a dying object's cargo sees.
awk '
  /^void C4Object::AssignDeath\(bool fForced\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_assign_death.inc"

awk '
  /^void C4Object::AssignRemoval\(bool fExitContents\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/object_assign_removal.inc"

awk '
  /^void C4Effect::Execute\(C4Object \*pObj\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Effect.cpp" > "$gen/effect_execute.inc"

awk '
  /^int32_t C4Effect::Check\(C4Object \*pForObj, const char \*szCheckEffect/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Effect.cpp" > "$gen/effect_check.inc"

# The complete effect lifecycle differential compiles the production list
# constructor and every mutation-bearing lifecycle method beside one compact
# callback scaffold.  Keep each body as a mechanical source extract: the
# fixture is specifically intended to catch pointer-walk, callback-order and
# priority-state changes that a transcription could accidentally normalize.
awk '
  /^C4Effect::C4Effect\(C4Object \*pForObj, const char \*szName,/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Effect.cpp" > "$gen/effect_lifecycle_constructor.inc"

while read -r lifecycle_method lifecycle_slug; do
  awk -v method="$lifecycle_method" '
    $0 ~ ("^(C4AulScript \\*|void )C4Effect::" method "\\(") { p = 1 }
    p { print }
    p && /^}$/ { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$src/C4Effect.cpp" > "$gen/effect_lifecycle_$lifecycle_slug.inc"
done <<'EOF'
GetCallbackScript get_callback_script
ClearPointers clear_pointers
Kill kill
ClearAll clear_all
DoDamage do_damage
TempRemoveUpperEffects temp_remove_upper_effects
TempReaddUpperEffects temp_readd_upper_effects
EOF

# 3n. Lift C4MouseControl::UpdateCursorTarget's OCF priority cascade. It is a
#     run of UNCONDITIONAL overwrites, so the LAST matching rule wins, not the
#     first — Enter, then Grab/Ungrab, then Carryable, then Chop's reduced
#     range, then Entrance, Build, the two Selects and finally Attack. A port
#     that turned it into a first-match chain would show the wrong cursor for
#     every object matching more than one rule, which is most of them.
#
#     Extracted as a fragment between its own comment markers, the way the
#     DFA_PUSH/PULL/FIGHT direction blocks already are: the surrounding function
#     handles regions, captions and drag state that this section does not pin.
awk '
  /^\t\t\/\/ Enter \(containers\)$/ { p = 1 }
  p { print }
  /^\t\t\t\t\tCursor = C4MC_Cursor_Attack;$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_cursor_cascade.inc"

# 3o. Lift the mouse target lookup and release-time command paths used by the
#     clonk-org/clonk-rs#521 differential.  FindVisObject, GetTargetObject and
#     GetOCFForPos are complete production functions.  The event scaffold also
#     compiles the complete cache-clearing, Tick5 update, right-up, put-target,
#     put-release, DragMoving and packet-enqueue methods.  Bounded fragments
#     cover UpdateCursorTarget's acquisition/region/fog prefix (the later
#     cursor-art cascade is covered above), Move's active and position/control
#     prefixes plus Button_None dispatch, and Move's fog pre-dispatch guard.
#     Their source comment/statement boundaries make extraction drift fail.
awk '
  /^C4Object \*C4Game::FindVisObject\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Game.cpp" > "$gen/mouse_find_vis_object.inc"

awk '
  /^C4Object \*C4MouseControl::GetTargetObject\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_get_target_object.inc"

awk '
  /^void C4Object::GetOCFForPos\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Object.cpp" > "$gen/mouse_get_ocf_for_pos.inc"

awk '
  /^int C4ObjectList::ClearPointers\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/mouse_object_list_clear_pointers.inc"

awk '
  /^int32_t C4ObjectList::ObjectNumber\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4ObjectList.cpp" > "$gen/mouse_object_number.inc"

awk '
  /^C4ControlPlayerSelect::C4ControlPlayerSelect\(/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Control.cpp" > "$gen/mouse_player_select_constructor.inc"

awk '
  /^C4ControlPlayerCommand::C4ControlPlayerCommand\(/ { p = 1 }
  p { print }
  p && /iData\(iData\), iAddMode\(iAddMode\) \{\}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Control.cpp" > "$gen/mouse_player_command_constructor.inc"

while read -r method slug; do
  awk -v method="$method" '
    $0 ~ ("^(void|bool) C4MouseControl::" method "\\(") { p = 1 }
    p { print }
    p && /^}$/ { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$src/C4MouseControl.cpp" > "$gen/mouse_$slug.inc"
done <<'EOF'
Execute execute
ClearPointers clear_pointers
UpdatePutTarget update_put_target
ButtonUpDragMoving button_up_drag_moving
DragMoving drag_moving
SendCommand send_command
RightUpDragNone right_up_drag_none
RightUp right_up
SendPlayerSelectNext send_player_select_next
EOF

awk '
  /^void C4MouseControl::UpdateCursorTarget\(\)$/ { fn = 1 }
  fn && /^\t\/\/ Scrolling: no other target$/ { p = 1 }
  p { print }
  p && /^\tif \(TargetObject && FogOfWar && .*TargetObject = nullptr;$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_update_target_acquisition.inc"

awk '
  /^void C4MouseControl::Move\(/ { fn = 1 }
  fn && /^\t\/\/ Active$/ { p = 1 }
  p { print }
  p && /^\tif \(!Active \|\| !fMouseOwned\) return;$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_move_active.inc"

awk '
  /^\t\t\/\/ Position$/ { p = 1 }
  p { print }
  p && /^\t\tShiftDown = false; if \(dwKeyFlags & MK_SHIFT\) ShiftDown = true;$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_move_position_control.inc"

awk '
  /^\tcase C4MC_Button_None:$/ { p = 1 }
  p { print }
  p && /^\t\tbreak;$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_move_none_dispatch.inc"

awk '
  /^\t\t\/\/ Blocked by fog of war: evaluate button up, dragging and region controls only$/ { p = 1 }
  p { print }
  p && /^\t\t}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_fog_button_up.inc"

awk '
  /^\tcase C4MC_Button_RightUp:[[:space:]]+RightUp\(\); break;$/ { print; found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4MouseControl.cpp" > "$gen/mouse_right_up_dispatch.inc"

# 3p. Lift the C4GameSave save-policy query functions and each specialization's
#     overrides. Every one is a pure function of Sync, fInitial and the ctor
#     flags, so the whole five-variant decision matrix extracts as header
#     fragments and needs no engine link -- the out-of-line virtuals
#     (AdjustCore/WriteDesc/SaveComponents/OnSaving) are never called here.
#
#     This is the matrix that decides what a saved file actually contains, and
#     several entries are non-obvious inversions: GetKeepTitle is !IsExact() so
#     a scenario save keeps the localized title a savegame deletes, and
#     C4GameSaveScenario overrides GetClearOrigin to a flat false so that a
#     scenario save NEVER clears an existing origin even when it is not saving
#     one -- the base class would clear it.
awk '
  /^\tvirtual bool GetSaveRuntimeData\(\) \{ return !fInitial; \}/ { p = 1 }
  p { print }
  p && /^\tvirtual bool GetSaveScriptPlayerFiles\(\) \{ return IsExact\(\); \}/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.h" > "$gen/game_save_base_queries.inc"

awk '
  /^\tvirtual bool GetSaveOrigin\(\) override \{ return fSaveOrigin; \}$/ { p = 1 }
  p { print }
  p && /^\tvirtual bool GetSaveScriptPlayerFiles\(\) override \{ return true; \}/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.h" > "$gen/game_save_scenario_queries.inc"

awk '
  /^\tvirtual bool GetSaveOrigin\(\) override \{ return true; \} \/\/ origin must be saved in savegames$/ { p = 1 }
  p { print }
  p && /^\tvirtual bool GetSaveUserPlayerFiles\(\) override \{ return false; \}/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.h" > "$gen/game_save_savegame_queries.inc"

awk '
  /^\tvirtual bool GetSaveDesc\(\) override \{ return false; \} \/\/ desc is saved by external call/ { p = 1 }
  p { print }
  p && /^\tvirtual bool GetCopyScenario\(\) override \{ return fCopyScenario; \}/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.h" > "$gen/game_save_record_queries.inc"

awk '
  /^\tvirtual bool GetSaveOrigin\(\) override \{ return true; \} \/\/ clients must know where to get music/ { p = 1 }
  p { print }
  p && /^\tvirtual bool GetCopyScenario\(\) override \{ return false; \}/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.h" > "$gen/game_save_network_queries.inc"

# 3p. Lift WildcardMatch, the matcher behind C4Group entry access, child-group
#     opening and every stock sort list. It is a self-contained backtracking
#     loop over two C strings and needs nothing but tolower.
#
#     The details that a rewrite loses: it is CASE-INSENSITIVE through tolower,
#     `?` does NOT match the end of the string (the empty-`pPos` break is tested
#     before it), a trailing `*` matches the empty remainder, and the loop
#     condition `*pWild || pLWild` keeps running on backtracking state alone
#     after the pattern is exhausted. A port that treats `*` as a literal still
#     passes every exact-name lookup, so only a `*` case can catch it.
awk '
  /^bool WildcardMatch\(const char \*szWildcard, const char \*szString\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/StdFile.cpp" > "$gen/wildcard_match.inc"

# 3q. Lift C4ConfigGeneral::GetLanguageSequence. The condensing rule it relies
#     on lives in the C4Strings helpers -- SCopySegment does the comma split,
#     the whitespace skip and, crucially, the TRUNCATION to two characters, so
#     `English` becomes `En` rather than being rejected -- and those are no
#     longer lifted: `src/C4Strings.cpp` is linked whole (see the compile step),
#     so this caller reaches the real ones.
awk '
  /^int C4ConfigGeneral::GetLanguageSequence\(const char \*strSource, char \*strTarget\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Config.cpp" > "$gen/config_language_sequence.inc"

# 3r. Lift C4Config::AdaptToCurrentVersion, the config post-init migration
#     C4Config::Load runs right after DeterminePaths (C4Config.cpp:1110-1112).
#     It is taken whole rather than reimplemented because the interesting parts
#     are its exact shape: the 347 arm FALLS THROUGH into 346, the address
#     rewrites are SEqual comparisons against retired defaults rather than
#     prefix matches, and the `<= 359` block runs independently of the switch.
#     The trailing `#ifdef USE_RUST_CONFIG` SyncRust() call compiles out here,
#     exactly as it does in an engine built without that bridge.
#     The retired-default addresses it rewrites TO are macros in C4Config.h;
#     lift those too, so a server move in the header cannot silently leave the
#     golden pinning an address the engine no longer uses.
awk '
  /^#define C4CFG_OfficialLeagueServer/ { p = 1 }
  p { print }
  p && /^#define C4CFG_UpdateServer/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Config.h" > "$gen/config_server_macros.inc"

awk '
  /^void C4Config::AdaptToCurrentVersion\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Config.cpp" > "$gen/config_adapt_version.inc"

# 3t. Lift C4GameSave::SaveRuntimeData, the ordered component sweep the save
#     policy queries drive. The order is the parity fact, and two of its rules
#     read backwards: Title is written only when the save is NOT exact, and a
#     failing Script/Title/Info write is `nofail` — it logs and carries on —
#     while a failing Landscape/Objects/Teams write aborts the whole save.
awk '
  /^bool C4GameSave::SaveRuntimeData\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.cpp" > "$gen/game_save_runtime_data.inc"

# 3v. Lift C4GameSave::SaveCore and the three AdjustCore overrides that run at
#     the end of it. The parity fact is the ORDER of the two: SaveCore zeroes
#     NetworkGame for every save, and C4GameSaveNetwork::AdjustCore then sets
#     it back — so the field's final value is decided by the sequence, not by
#     either function alone. SaveCore also leaves NoInitialize and SaveGame
#     untouched for an INITIAL save, and rewrites the title only when
#     GetKeepTitle() is false.
awk '
  /^bool C4GameSave::SaveCore\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.cpp" > "$gen/game_save_core.inc"

awk '
  /^int GetTrailingNumber\(const char \*strString\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/StdFile.cpp" > "$gen/get_trailing_number.inc"

for cls in Savegame Record Network; do
  awk -v cls="$cls" '
    $0 ~ ("^void C4GameSave" cls "::AdjustCore\\(C4Scenario &rC4S\\)$") { p = 1 }
    p { print }
    p && /^}$/ { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$src/C4GameSave.cpp" > "$gen/game_save_adjust_$(echo "$cls" | tr 'A-Z' 'a-z').inc"
done

# 3w. Lift C4GameSave::SaveScenarioSections together with the C4ScenarioSection
#     constructor that builds the list it walks. `save_runtime_sequence`
#     records that an exact save *reaches* this step; what the step does is
#     stubbed there, and nothing else pins it.
#
#     Three rules live in these two bodies and none is visible from the call
#     site:
#       * the constructor PREPENDS (`pNext = Game.pScenarioSections`), so the
#         sweep runs in reverse construction order and the implicit current
#         node -- created at the first section switch, hence newest -- is
#         visited FIRST;
#       * the current section is deleted and never re-added, whether or not it
#         is modified, because its state is implied by CurrentScenarioSection
#         and the main landscape/object data;
#       * a modified section deletes before it adds and the Add result is
#         DISCARDED, so a broken temp file leaves that entry absent instead of
#         failing the save.
#
#     The constructor also folds an empty or "Main" name onto C4ScenSect_Main,
#     which is what decides the entry name for the implicit root.
awk '
  /^bool C4GameSave::SaveScenarioSections\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4GameSave.cpp" > "$gen/game_save_scenario_sections.inc"

awk '
  /^const char \*C4ScenSect_Main = "main";$/ { print; ++found; next }
  /^C4ScenarioSection::C4ScenarioSection\(char \*szName\)$/ ||
  /^const char \*C4ScenarioSection::GetName\(\) const$/ ||
  /^const char \*C4ScenarioSection::GetTempFilename\(\) const$/ { p = 1; ++found }
  p { print; if ($0 ~ /^}$/) p = 0 }
  END { if (found != 4) exit 1 }
' "$src/C4Scenario.cpp" > "$gen/scenario_section_ctor.inc"

# Each entry name is `SCopy` of the pattern, `SDelete` of the wildcard, then
# `SInsert` of the section name at that position -- so the composed name is
# `Sect` + section name + `.c4g` only because of where the splice lands. All
# four come from the linked `src/C4Strings.cpp`, so the real `Sect*.c4g` in the
# real C4Components.h and the splice that rewrites it stay honest together.

# 3r. Lift C4Value::operator== whole. It is a nested switch on the LEFT tag and
#     then the right, which is what makes it asymmetric: the object arm demands
#     an equal tag as well as an equal payload, so `nil == object_zero` is true
#     while `object_zero == nil` is false. Generating the full ordered matrix is
#     the point -- that turns out to be the ONLY asymmetric pair, and a port
#     that assumed the differing arms produce more asymmetry than they do would
#     look reasonable and be wrong.
awk '
  /^bool C4Value::operator==\(const C4Value &Value2\) const$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_operator_equal.inc"

# C4ValueHash does not use operator== for its keys. Its KeyEqual calls
# Equals(MAXSTRICT), whose Bool arm compares truthiness rather than the raw
# union word. Lift that complete dispatcher too so the map-key lookup matrix
# below exercises the native distinction instead of restating it.
awk '
  /^bool C4Value::Equals\(const C4Value &other, C4AulScriptStrict strict\) const$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_equals.inc"

# C4Value's element-reference machinery and C4ValueList's relocation path are
# stateful, so lift the complete production method bodies used by the bounded
# runtime-operation table. The scaffold supplies only allocation/reference
# counters, object/string stubs for unreachable switch arms, and JSON output.
awk '
  /^C4Value::~C4Value\(\)$/ ||
  /^C4Value &C4Value::operator=\(const C4Value &nValue\)$/ ||
  /^void C4Value::AddDataRef\(\)$/ ||
  /^void C4Value::DelDataRef\(/ ||
  /^void C4Value::Set\(C4V_Data nData, C4V_Type nType\)$/ ||
  /^void C4Value::Set0\(\)$/ ||
  /^void C4Value::CheckRemoveFromMap\(\)$/ ||
  /^void C4Value::Move\(C4Value \*nValue\)$/ ||
  /^void C4Value::GetContainerElement\(/ ||
  /^void C4Value::SetArrayLength\(/ ||
  /^const C4Value &C4Value::GetRefVal\(\) const$/ ||
  /^C4Value &C4Value::GetRefVal\(\)$/ ||
  /^void C4Value::AddRef\(/ ||
  /^void C4Value::DelRef\(/ { p = 1; ++found }
  p { print }
  p && /^}$/ { p = 0 }
  END { if (found != 14) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_runtime_core.inc"

# Inline mutation/dispatch entry points used by the stateful conversion rows.
# Keep their class bodies exact as well as the out-of-line converter table.
awk '
  /^\tvoid SetObject\(C4Object \*Obj\)/ { print; ++found; next }
  /^\tvoid Deref\(\)/ { print; ++found; next }
  /^\tinline bool ConvertTo\(/ { p = 1; ++found }
  p { print }
  p && /^\t}$/ { p = 0 }
  END { if (found != 3) exit 1 }
' "$src/C4Value.h" > "$gen/c4value_runtime_inline.inc"

# Lift the complete conversion dispatch implementation and table. The
# stateful rows exercise its mutations (including the reference-dereference
# retry), not just the already-golden 9x9 classification.
awk '
  /^static bool FnCnvDirectOld\(/ { p = 1 }
  p { print }
  p && /^#undef CnvDeref$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_runtime_conversion.inc"

# Serialized object references resolve active objects first, then the inactive
# list. Lift the complete method so explicit C4V_C4ObjectEnum and legacy
# C4V_Any+offset rows execute the production branch order.
awk '
  /^void C4Value::DenumeratePointer\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_runtime_denumerate.inc"

# A missing legacy C4V_Any pointer passes through GuessType. Keep that fallback
# native too, including the exact packed-ID classifier it calls, so the
# denumeration table does not restate even its final type transition.
awk '
  /^C4V_Type C4Value::GuessType\(\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_runtime_guess_type.inc"

awk '
  /^bool LooksLikeID\(C4ID id\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Id.cpp" > "$gen/c4value_runtime_looks_like_id.inc"

awk '
  /^C4ValueList::C4ValueList\(const std::int32_t size\)$/ ||
  /^C4ValueList::C4ValueList\(const C4ValueList &other\)$/ ||
  /^C4ValueList &C4ValueList::operator=\(const C4ValueList &other\)$/ ||
  /^C4Value &C4ValueList::GetItem\(std::int32_t index\)$/ ||
  /^void C4ValueList::SetSize\(const std::int32_t size\)$/ ||
  /^C4ValueArray::C4ValueArray\(\)$/ ||
  /^C4ValueArray::C4ValueArray\(int32_t inSize\)$/ ||
  /^C4ValueArray::C4ValueArray\(const C4ValueArray &Array2\)$/ ||
  /^C4ValueArray::~C4ValueArray\(\)/ ||
  /^C4ValueArray \*C4ValueArray::SetLength\(int32_t size\)$/ ||
  /^bool C4ValueArray::hasIndex\(const C4Value &index\) const$/ ||
  /^C4Value &C4ValueArray::operator\[\]\(const C4Value &index\)$/ { p = 1; ++found }
  p {
    print
    braces = $0
    opened = gsub(/{/, "", braces)
    closed = gsub(/}/, "", braces)
    depth += opened - closed
    if ($0 ~ /^[[:space:]]*{/ || $0 ~ /\{\}[[:space:]]*$/) started = 1
    if (started && depth == 0) { p = 0; started = 0 }
  }
  END { if (found != 12) exit 1 }
' "$src/C4ValueList.cpp" > "$gen/c4value_runtime_list.inc"

# This template is the native copy-on-write and element-reference counter used
# by C4ValueArray. Strip only the header guard/includes and keep the class body
# byte-for-byte.
awk '
  /^template <typename T>$/ { p = 1 }
  p { print }
' "$src/C4ValueStandardRefCountedContainer.h" > "$gen/c4value_runtime_refcount.inc"

# 3u. Lift the C4Value type-character map, both directions. It is what every
#     saved script value leads with, and three of its pairs differ only by
#     CASE — i/I is Int vs C4ID, o/O is object vs enumerated object, a/A is
#     array vs any — so a slip silently changes a value's type on reload
#     rather than failing. The reverse direction answers C4V_Any for anything
#     it does not know, so a corrupted tag is read as nil, never rejected.
awk '
  /^char GetC4VID\(const C4V_Type Type\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_get_id.inc"

awk '
  /^C4V_Type GetC4VFromID\(const char C4VID\)$/ { p = 1 }
  p { print }
  p && /^}$/ { found = 1; exit }
  END { if (!found) exit 1 }
' "$src/C4Value.cpp" > "$gen/c4value_from_id.inc"

# 4. Compile the oracle against the real C4Random.h (no DEBUGREC), the real
#    C4ScriptKiller.h/C4LandscapePath.h/C4ActionDirection.h/
#    C4SolidMaskBitmap.h production helpers, and the generated header/table;
#    then run it to produce the golden JSON. The pinned ExecuteScan body keeps
#    its intentional nested-if formatting, so suppress only that style warning.
# C4Group is the first section whose subject is BYTES rather than a call
# order, so its oracle cannot be a recorder: `src/C4Group.cpp` and the file
# layer beneath it are linked whole and driven over real temporary files.
#
# That link is why the C4Strings helpers are no longer lifted into
# `oracle_main.cpp` -- the lifted copies would be duplicate symbols beside the
# real `C4Strings.cpp`. Everything that used them now calls the real thing.
#
# SHA1 is the one stub. `C4Group` reaches it only through the `Original` author
# signature, which no section verifies, and linking it for real would drag in
# OpenSSL.
cat > "$gen/stdsha1_stub.cpp" <<'STUB'
#include <StdSha1.h>
struct StdSha1::Impl {};
StdSha1::StdSha1() = default;
StdSha1::~StdSha1() = default;
void StdSha1::Update(const void *, std::size_t) {}
void StdSha1::GetHash(void *) {}
STUB

# `-std=c++23`: C4Group.h:296,301 use std::to_underlying, which C++20 lacks.
#     Verified to reproduce the committed golden byte-for-byte, so the other
#     sections are unaffected by the bump.
# `-DZLIB_CONST`: StdGzCompressedFile.cpp:108,275 assign a `const uint8_t *` to
#     zlib's `Bytef *`, which only compiles under zlib's const-correct typing.
cxx="${CXX:-clang++}"
"$cxx" -std=c++23 -O0 -DZLIB_CONST \
  -Wno-dangling-else \
  -I"$gen" -I"$src" \
  "$here/oracle_main.cpp" "$gen/sine_table.cpp" "$gen/stdsha1_stub.cpp" \
  "$src/C4Group.cpp" "$src/C4Strings.cpp" "$src/StdFile.cpp" \
  "$src/CStdFile.cpp" "$src/StdGzCompressedFile.cpp" "$src/StdBuf.cpp" \
  "$src/C4InputValidation.cpp" \
  -lz \
  -o "$gen/oracle"

"$gen/oracle" > "$out"
echo "wrote $out ($(wc -c < "$out") bytes) from $oracle_revision ($oracle_commit)"
