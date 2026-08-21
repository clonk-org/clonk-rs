/*-- Simple gather task for an idle crew member --*/

// clonk-rs divergence from LegacyClonk (clonk-org/clonk-rs#334). LegacyClonk
// has no standing order of this shape: a spare Clonk is either driven by hand
// or left as a spare, which is why a second crew member is mostly a backup
// rather than something to do. This adds the selection half of a "go and fetch
// all of these" order, in script, so nothing in the synchronised engine
// changes.
//
// The constraint the request is built around is reachability *both ways*: an
// order is only offered for an item the Clonk can walk to and carry back from.
// `PathFree` is the engine's own line-of-walk test, so a candidate list built
// from it cannot propose a fetch that strands the Clonk on the far side of a
// chasm.
//
// This file is deliberately the *selection* half only. `ClonkRsGatherOrder` is
// a thin wrapper over `AddCommand`; the menu that would offer it is not written
// here, because which items appear, how an order is cancelled and what happens
// when the base is destroyed are gameplay decisions rather than mechanical
// ones.

#strict

// Loose objects of `item` this Clonk could fetch and bring back to `base`.
//
// "Loose" means uncontained: an item already inside a container or another
// Clonk is somebody's, not litter. `base` may be nil, in which case only the
// outward path is required and the caller is asking "what can it reach".
global func ClonkRsGatherCandidates(object clonk, id item, object base)
{
  var candidates = CreateArray();
  if (!clonk)
    return(candidates);

  var clonk_x = GetX(clonk);
  var clonk_y = GetY(clonk);
  var candidate, x, y;
  for (candidate in FindObjects(Find_ID(item)))
  {
    if (!candidate)
      continue;
    // Uncontained only: an item in a container belongs to someone.
    if (Contained(candidate))
      continue;
    x = GetX(candidate);
    y = GetY(candidate);
    // Out: the Clonk has to be able to walk to it.
    if (!PathFree(clonk_x, clonk_y, x, y))
      continue;
    // Back: and to carry it home again. Without this an order can strand a
    // Clonk somewhere it can reach but not leave.
    if (base && !PathFree(x, y, GetX(base), GetY(base)))
      continue;
    candidates[GetLength(candidates)] = candidate;
  }
  return(candidates);
}

// Queue the fetch-and-return orders for everything the selection allows.
// Returns how many items were ordered, so a caller can tell "nothing
// reachable" from "nothing there".
global func ClonkRsGatherOrder(object clonk, id item, object base)
{
  var candidates = ClonkRsGatherCandidates(clonk, item, base);
  var candidate;
  for (candidate in candidates)
  {
    AddCommand(clonk, "Get", candidate);
    if (base)
      AddCommand(clonk, "Enter", base);
  }
  return(GetLength(candidates));
}

// The distinct item types this Clonk could fetch and bring back, as
// [id, count] rows. This is the list the menu is built from.
//
// One row per type rather than one per object: a menu built straight off the
// object list is a list of litter rather than a list of orders, and the count
// is the only thing on a row that tells the player whether the order is worth
// giving at all.
//
// `Find_Category(C4D_Object)` is what separates that litter from the crew
// member standing in it and the building it would be carried to. Both of those
// are uncontained and both are trivially reachable, so nothing else here would
// keep them off the menu.
global func ClonkRsGatherTypes(object clonk, object base)
{
  var rows = CreateArray();
  if (!clonk)
    return(rows);

  var seen = CreateArray();
  var candidate, item, i, known, count, row;
  for (candidate in FindObjects(Find_Category(C4D_Object)))
  {
    if (!candidate)
      continue;
    // Uncontained only, for the same reason as the candidate list.
    if (Contained(candidate))
      continue;
    item = GetID(candidate);
    known = false;
    for (i = 0; i < GetLength(seen); i++)
      if (seen[i] == item)
        known = true;
    if (known)
      continue;
    seen[GetLength(seen)] = item;
    // Reuse the reachability filter rather than repeating it here, so the
    // count on a row can never disagree with what the order actually fetches.
    count = GetLength(ClonkRsGatherCandidates(clonk, item, base));
    if (!count)
      continue;
    row = CreateArray(2);
    row[0] = item;
    row[1] = count;
    rows[GetLength(rows)] = row;
  }
  return(rows);
}
