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
// A right mouse button is still a hidden control, so the left and right
// controls step the selected value too. They are dead keys here: once
// Columns == 1, C4Menu::Control gives them exactly the deltas Up/Down already
// carry (C4Menu.cpp:433-457). The engine offers them to this script first
// through OnMenuStep and falls back to its own selection move when we decline,
// so every other menu in every pack is untouched.
//
// Deliberately NOT changed: Increase and Decrease themselves, so the step
// arithmetic and its BoundBy clamp stay exactly as shipped (Menu.c4d/
// Script.c:178-198), and the non-selectable branch keeps its inert ShowMenu
// command and greyed symbol.

#strict 2

// `nowarn`: MS4C ships with ClonkMars, so every other scenario links this
// script with the target absent (C4AulLink.cpp:42-49).
#appendto MS4C nowarn

// Row index -> range key for the page on screen, so a step control can find
// the range under the selection. Rebuilt by ShowMenu on every render; rows
// that are not ranges stay empty.
local row_keys;

// Set only across the CreateMenu that re-renders a page. Replacing a live
// menu asks the owner whether the close is denied (C4Script.cpp:1525 ->
// C4Menu::TryClose -> C4ObjectMenu::IsCloseDenied, C4ObjectMenu.cpp:57-76),
// and Menu2 answers that question by aborting the whole template. Row
// commands never hit it because C4Menu::Enter closes a non-permanent menu
// before running them (C4Menu.cpp:517); a step control runs with the menu
// still open, so it does.
local menu_rendering;

// Row index -> the caption its range showed, so an undo entry can name what
// it will take back.
local row_names;

// One entry per adjustment that actually moved a value:
// [path, key, value before, row caption, row index].
local adjust_history;

// ShowMenu, rebuilt rather than inherited so the closing row can say which of
// its two jobs it is doing. Mirrors Menu.c4d/Script.c:27-56 exactly otherwise.
// The MS4C_* index constants live in Menu2's own System.c4g, which is not
// registered when this script parses, so the literal indices are spelled out
// against Menu.c:25-34: menu = [symbol, caption, hash, sequence] and
// value = [type, cond, name, id, data]; types are Bool 0, Enum 1, Range 2,
// Submenu 3 (Menu.c:5-8).
private func ShowMenu(int iSelection)
{
  // PushBack writes through a reference (Menu2 System.c4g/Helpers.c:6-9), so
  // the history has to be an array before the first adjustment reaches it.
  if (!adjust_history)
    adjust_history = CreateArray();
  var currentMenu;
  if (aCurrentPath && aCurrentPath != [])
    currentMenu = GetSubmenu(aMenu, aCurrentPath);
  else
    currentMenu = aMenu;
  menu_rendering = true;
  CreateMenu(currentMenu[0], pMenuObject, this, 0, currentMenu[1], 0, 1);
  menu_rendering = false;
  row_keys = CreateArray();
  row_names = CreateArray();
  var i = 0;
  for (var Key in currentMenu[3])
  {
    var value = MenuGet(currentMenu, 0, Key);
    var type = value[0];
    if (type == 0)
      ShowBool(Key, value[1], value[2], value[3], value[4], i);
    else if (type == 1)
      ShowEnum(Key, value[1], value[2], value[3], value[4], i);
    else if (type == 2)
      ShowRange(Key, value[1], value[2], value[3], value[4], i);
    else if (type == 3)
      ShowSubmenu(Key, value[1], value[2], value[3], value[4], i);
  }
  ShowUndoRow(i);
  ShowClosingRow(i);
  SelectMenuItem(iSelection, pMenuObject);
}

// A step control and a right mouse button are both invisible until someone
// tells you about them. This row is neither: it is on screen from the first
// change, it names what it will take back, and a plain activation runs it.
private func ShowUndoRow(&i)
{
  if (!GetLength(adjust_history))
    return;
  var entry = adjust_history[GetLength(adjust_history) - 1];
  var szCaption = Format("$MenuUndo$", entry[3]);
  AddMenuItem(szCaption, Format("Undo(%d)", i++), MS4C, pMenuObject, 0, 0,
    szCaption, 2, 2);
}

// The shipped page ends in one row captioned "Finished" whichever of its two
// jobs it is about to do: step back out of a submenu, or hand the whole
// template to the callback (Menu.c4d/Script.c:54,206-228). Name the job.
private func ShowClosingRow(&i)
{
  var szCaption = "$MenuDone$";
  if (aCurrentPath && aCurrentPath != [])
    szCaption = "$MenuBack$";
  AddMenuItem(szCaption, "Finished()", MS4C, pMenuObject, 0, 0, szCaption, 2, 3);
  i++;
}

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
  row_keys[i] = Key;
  row_names[i] = szName;
  AddMenuItem(Format("%s%s", szName, szSteps), Format("Adjust(%%d,%d,%%d)", i++),
    idItem, pMenuObject, iValue, Key, szInfo);
}

// The engine offers COM_MenuLeft/Right here before turning them into a
// selection move. Claim them only over a range row, so the arrows keep
// navigating everywhere else.
public func OnMenuStep(int iDelta, object pMenuObject)
{
  var iRow = GetMenuSelection(pMenuObject);
  if (iRow < 0 || iRow >= GetLength(row_keys))
    return false;
  // A hole means the row is not a range. Menu2 keys are C4IDs or strings, so
  // a falsy key cannot be a real one (Menu.c:198-211).
  var Key = row_keys[iRow];
  if (!Key)
    return false;
  Adjust(Key, iRow, iDelta < 0);
  return true;
}

public func MenuQueryCancel()
{
  // Re-rendering a page is not the player cancelling.
  if (menu_rendering)
    return 0;
  // Escape abandons the order from any page. Shipped Menu2 pops one level
  // instead, so Escape on a submenu reopened the page above it and the
  // ordering UI could not be dismissed from there at all
  // (Menu.c4d/Script.c:208-212,230-235).
  aCurrentPath = CreateArray();
  CloseMenu(pMenuObject);
  Finished(true);
  return 1;
}

// Reached as the menu command on the MS4C helper itself, which ShowMenu passes
// to CreateMenu as pCommandObj (Menu.c4d/Script.c:34-35). Increase and
// Decrease keep the shipped BoundBy clamp, so a step that saturates changes
// nothing and must not become an undo entry.
protected func Adjust(Key, int iSelection, int fRight)
{
  var szName = row_names[iSelection];
  var aPath = CopyPath(aCurrentPath);
  var iBefore = MenuGet(aMenu, aCurrentPath, Key)[4][3];
  // The shipped step arithmetic, minus its re-render: the history entry has
  // to exist before the page is drawn or the undo row appears one activation
  // late (Increase/Decrease at Menu.c4d/Script.c:178-198 render immediately).
  if (fRight)
    DecreaseRange(MenuGet(aMenu, aCurrentPath, Key)[4]);
  else
    IncreaseRange(MenuGet(aMenu, aCurrentPath, Key)[4]);
  if (MenuGet(aMenu, aPath, Key)[4][3] != iBefore)
    PushBack([aPath, Key, iBefore, szName, iSelection], adjust_history);
  return ShowMenu(iSelection);
}

protected func Undo(int iSelection)
{
  if (!GetLength(adjust_history))
    return ShowMenu(iSelection);
  var entry = adjust_history[GetLength(adjust_history) - 1];
  DeleteLast(adjust_history);
  MenuGet(aMenu, entry[0], entry[1])[4][3] = entry[2];
  // Land back on the row that changed rather than wherever the undo row was.
  return ShowMenu(entry[4]);
}

// aCurrentPath is mutated in place by OpenMenu and Finished
// (Menu.c4d/Script.c:200-212), so an undo entry needs its own copy.
private func CopyPath(array aPath)
{
  var aCopy = CreateArray();
  if (aPath)
    for (var Key in aPath)
      PushBack(Key, aCopy);
  return aCopy;
}
