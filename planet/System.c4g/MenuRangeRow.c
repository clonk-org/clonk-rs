/*-- Menu2: one row per adjustable value --*/

// clonk-rs divergence from LegacyClonk. ClonkMars' bundled Menu2 helper
// expands every range choice into three rows -- the value, an unlabelled
// "Increase by 1" and an unlabelled "Decrease by 1" (ClonkMars.c4d/
// Helpers.c4d/Menu2.c4d/Menu.c4d/Script.c:108-125). The Cerberus Fossae order
// page therefore listed 16 rows for five products, in a menu the engine draws
// as one narrow column of single-line rows (C4Menu.cpp:359-365,650-664).
//
// This append collapses a range to one row: the primary activation adds a
// step, the secondary one takes a step back. C4Menu::Enter runs C4MenuItem::
// Command2 on a right enter (C4Menu.cpp:512-514), which the engine composes
// itself out of the same command string (C4Script.cpp:1630-1670) and which
// the player reaches with the right mouse button (C4Menu.cpp:228-232), with
// [Special2] (C4Menu.cpp:1053) or through COM_MenuEnterAll (C4Menu.cpp:440) --
// the same two activations the bottom key strip already advertises
// (C4Menu.cpp:846-880). The engine's own base-purchase menu next door already
// spends those two activations on two quantities of one product
// (C4ObjectMenu.cpp:246-271), so one row per product is not a new layout.
//
// The row states which steps it will take, and offers only the ones still
// inside [min, max], so a value sitting on a limit says so instead of
// promising a click that would do nothing.
//
// Deliberately NOT changed: Increase and Decrease themselves, so the step
// arithmetic and its BoundBy clamp stay exactly as shipped (Menu.c4d/
// Script.c:178-198), and the non-selectable branch keeps its inert ShowMenu
// command and greyed symbol.

#strict 2

// `nowarn`: MS4C ships with ClonkMars, so every other scenario links this
// script with the target absent (C4AulLink.cpp:42-49).
#appendto MS4C nowarn

private func ShowRange(Key, array aCond, string szName, id idItem, data, &i)
{
  // data = [min, max, step, current] (Menu2's own System.c4g/Menu.c:77-82).
  var iStep = data[2], iValue = data[3];
  var fUp = iValue < data[1], fDown = iValue > data[0];

  if (!Evaluate_MenuCond(aMenu, aCond))
  {
    var szGreyed = GreyString(szName);
    var pDummy = CreateDummy(idItem, false, false, false);
    AddMenuItem(szGreyed, Format("ShowMenu(%d)", i++), 0, pMenuObject, iValue,
      0, szGreyed, 4, pDummy);
    RemoveObject(pDummy);
    return;
  }

  // Both steps are always spelled out, so the secondary activation is visible
  // before it is ever used; a step outside [min, max] is greyed rather than
  // dropped. Menu rows are measured with markup checking on, which skips the
  // tags outright (CStdFont::GetTextExtent, StdFont.cpp:571-601; ported at
  // crates/clonk-app-menus/src/object_menu.rs:1215-1254,1386-1394), so the row
  // is exactly as wide on every value and the menu never resizes under a
  // pointer that is clicking it.
  var szUp = Format("+%d", iStep), szDown = Format("-%d", iStep);
  if (!fUp) szUp = GreyString(szUp);
  if (!fDown) szDown = GreyString(szDown);
  var szSteps = Format(" (%s/%s)", szUp, szDown);

  var szInfo;
  if (fUp && fDown)
    szInfo = Format("$MenuRangeBoth$", iValue, iStep, iStep);
  else if (fUp)
    szInfo = Format("$MenuRangeUp$", iValue, iStep);
  else if (fDown)
    szInfo = Format("$MenuRangeDown$", iValue, iStep);
  else
    szInfo = Format("$MenuRangeFixed$", iValue);

  // One command string, two commands: the engine rewrites the first "%d" to
  // "%s" for the typed parameter and hands the second specifier 0 on a left
  // and 1 on a right enter (C4Script.cpp:1630-1645), giving
  // Adjust(<Key>,<row>,0) and Adjust(<Key>,<row>,1). No C4MN_Add_Img* extra,
  // so the row keeps the product's own picture as its symbol, and iValue
  // keeps the engine's right-aligned "Nx" on the row it belongs to
  // (C4Menu.cpp:198-207).
  AddMenuItem(Format("%s%s", szName, szSteps), Format("Adjust(%%d,%d,%%d)", i++),
    idItem, pMenuObject, iValue, Key, szInfo);
}

// Reached as the menu command on the MS4C helper itself, which ShowMenu passes
// to CreateMenu as pCommandObj (Menu.c4d/Script.c:34-35).
protected func Adjust(Key, int iSelection, int fRight)
{
  if (fRight)
    return Decrease(Key, iSelection);
  return Increase(Key, iSelection);
}
