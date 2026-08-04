/*-- ClonkMars: pay for a supply order before it is delivered --*/

// clonk-rs divergence from LegacyClonk. The shipped commit spends the
// player's money one item at a time with no check and no report
// (content/ClonkMars.c4d/Structures.c4d/Base.c4d/Script.c:133-155):
//
//   * `Buy(node[0], owner, owner, pCapsule)` omits fShowErrors, so
//     C4Player::Buy takes the silent branch and neither IDS_PLR_NOWEALTH nor
//     its Error sound is ever produced (C4Player.cpp:849-853);
//   * the first item that does not fit makes the loop `return true`, which
//     abandons every product still to come — and the hash iterates in bucket
//     order (ClonkMars.c4d/System.c4g/HashTable.c:173-189), so which half of
//     the order arrives is unrelated to the order on screen;
//   * the capsule is created before any of that (Script.c:140), so the
//     one-capsule allowance is spent either way and the player waits out the
//     five-minute cooldown for a part-load.
//
// Cerberus Fossae starts the player on Wealth=30 against 186 clunkers of
// stock (01_Fossae.c4s/Scenario.txt:17,21), so filling the order page is the
// ordinary outcome rather than an edge case.
//
// This append prices the order first and refuses it whole when it cannot be
// paid for, says so, and reports what was sent when it can. It also passes
// fShowErrors, so anything that still slips through gets the engine's own
// message instead of silence.
//
// Deliberately NOT changed: CapsuleCheck and its refusals, CreateCapsule and
// where the capsule lands, the per-product stock re-clamp, and the SellOnly
// branch — all still exactly as shipped.

#strict 2

// `nowarn`: BASE ships with ClonkMars, so every other scenario links this
// script with the target absent (C4AulLink.cpp:42-49).
#appendto BASE nowarn

public func OrderCapsule(hash, object pClonk, bool fCanceled)
{
  if (fCanceled || !CapsuleCheck(pClonk))
    return;
  var iOwner = pClonk->GetOwner();
  if (HashGet(hash, "Sell") == "SellOnly")
    return !!CreateCapsule(iOwner);

  var objs = HashGet(hash, "Buy");
  var iTotal = OrderTotal(objs, iOwner);
  var iWealth = GetWealth(iOwner);
  if (iTotal > iWealth)
  {
    Sound("Error");
    Message("$MarsOrderTooExpensive$", pClonk, iTotal, iWealth);
    return;
  }

  var pCapsule = CreateCapsule(iOwner);
  if (!pCapsule)
    return;
  var iSent = 0;
  var iter = HashIter(objs);
  var node;
  while (node = HashIterNext(iter))
  {
    var iWanted = OrderQuantity(node, iOwner);
    for (var i = 0; i < iWanted; i++)
      if (Buy(node[0], iOwner, iOwner, pCapsule, true))
        iSent++;
  }
  // The commit and a cancel used to be equally silent, and an empty order
  // still spent the allowance (Menu.c4d/Script.c:215-221 builds a summary and
  // throws it away because ClonkMars passes iVerbose 0).
  Message("$MarsOrderSent$", pClonk, iSent, iTotal);
  return true;
}

// Menu2 asks the menu's owner for a figure to put in the C4MN_Extra_Value
// footer, and shipped ClonkMars asks for no footer at all
// (Menu.c4d/Script.c:34-35 passes iExtra 0), so the player composed an order
// with neither a total nor their own wealth on screen -- the wealth counter is
// hidden unless something arms it (C4Viewport.cpp:1286-1296), and
// C4MN_Extra_Value is what arms it (C4Menu.cpp:901-905). Answering here turns
// both on for the ordering menu only.
public func MenuFooterValue(hash, object pClonk)
{
  if (!pClonk)
    return;
  return [OrderTotal(HashGet(hash, "Buy"), pClonk->GetOwner())];
}

// What the order will actually cost, priced exactly as the rows are captioned
// (Base.c4d/Script.c:125 uses the same GetValue) and clamped to the stock the
// commit is about to re-clamp to anyway.
private func OrderTotal(objs, int iOwner)
{
  var iTotal = 0;
  if (!objs)
    return iTotal;
  var iter = HashIter(objs);
  var node;
  while (node = HashIterNext(iter))
    iTotal += OrderQuantity(node, iOwner) * GetValue(0, node[0], this, iOwner);
  return iTotal;
}

// The shipped stock re-clamp (Base.c4d/Script.c:146-147), so pricing and
// delivery cannot disagree about how many a product will yield.
private func OrderQuantity(node, int iOwner)
{
  var iStock = GetHomebaseMaterial(iOwner, node[0]);
  if (node[1] > iStock)
    return iStock;
  return node[1];
}
