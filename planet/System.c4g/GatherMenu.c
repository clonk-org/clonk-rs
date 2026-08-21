/*-- Gather order menu for an idle crew member --*/

// clonk-rs divergence from LegacyClonk (clonk-org/clonk-rs#334). The selection
// half lives in GatherTask.c; this is the row the player actually clicks.
//
// Separate file because `#appendto` applies to a whole script: GatherTask.c
// stays a set of global helpers callable from anywhere, and only this binds to
// the crew definition.
//
// The order is a ONE-SHOT batch. Everything reachable at the moment the row is
// chosen is queued, and the Clonk then goes idle; an item that appears later is
// not picked up. That keeps the whole feature inside the existing command
// queue, with no repeating effect to run while the player is elsewhere and
// nothing to cancel beyond the ordinary command controls.

#strict

#appendto CLNK nowarn

// One row per reachable type. The row is hidden entirely when there is nothing
// to fetch, rather than shown greyed: an order with no items is not a choice
// the player needs to see.
public func ContextGather(object pCaller)
{
  [$GatherTask$|Image=CLNK|Condition=ClonkRsCanGather]
  var base = FindBase(GetOwner(this));
  var rows = ClonkRsGatherTypes(this, base);
  var menu_id = GetID(this);
  CreateMenu(menu_id, this, this, 0, "$GatherTask$");
  var row, item, count;
  for (row in rows)
  {
    item = row[0];
    count = row[1];
    // The count rides on the menu item so the caption and the order can never
    // disagree: MenuSelection re-derives the fetch list from the same filter.
    AddMenuItem(Format("$GatherRow$", GetName(0, item), count),
      Format("ClonkRsGatherSelected(%i)", item), item, this, count);
  }
  return(1);
}

// Condition for the row above: only offer it when something is reachable.
public func ClonkRsCanGather(object pCaller)
{
  return(GetLength(ClonkRsGatherTypes(this, FindBase(GetOwner(this)))) > 0);
}

// Menu command: queue the one-shot batch for the chosen type.
public func ClonkRsGatherSelected(id item)
{
  return(ClonkRsGatherOrder(this, item, FindBase(GetOwner(this))));
}
