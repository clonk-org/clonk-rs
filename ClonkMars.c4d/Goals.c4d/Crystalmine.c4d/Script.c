/*-- Kristallabbau --*/

#strict
#include GOAL

local Sold;

public func IsFulfilled()
{
  // Erz noch nicht gezählt?
  var CrystalCnt = GetMaterialCount(Material("Crystal"));
  if (CrystalCnt == -1) return();
  // noch zu viel?
  if (CrystalCnt >= 150) return();
  // noch Kristalle abzubauen?
  return(!ObjectCount(CRYS));
}

public func IsFulfilledforPlr (plr)
{
  if (!IsFulfilled()) return ();
  var i = GetPlayerCount();
  while (i--)
    if (Local (GetPlayerByIndex (i)) > Local (plr)) return (0);
  return (1);
}

func SellID () { return (CRYS); }

public func PlayerHasSold (plr, obj)
{
  if (GetID (obj) == SellID ())
  {
    ++Local (plr);
    ++Sold;
  }
}

public func PlayerHasBought (plr, obj)
{
  if (GetID (obj) == SellID ())
  {
    --Local (plr);
    --Sold;
  }
}

protected func Activate(iPlayer)
{
  if (IsFulfilled ()) return (MessageWindow ("$String0$", iPlayer));
  // -50 statt -150 weil beim / 100 abgerundet wird
  MessageWindow (Format ("$String1$", Max (GetMaterialCount (Material ("Crystal")) - 50, 0) / 100, ObjectCount (CRYS), Local (iPlayer), Sold), iPlayer);
}
