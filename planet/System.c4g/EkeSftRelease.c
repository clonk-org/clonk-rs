/*-- Eke Reloaded: forward turn-key releases to the carried item --*/

// clonk-rs divergence from LegacyClonk. The Eke SFT forwards ControlLeft and
// ControlRight to its selected item (EkeReloaded.c4d/Creatures.c4d/SFT.c4d/
// Script.c:40-104) but has no release counterpart, so an item that latches a
// steering command never learns that the key went up. These appends complete
// the pair; every item without a *Released callback is unaffected, because
// ObjectCall on a missing function is a no-op.

#strict

#appendto SF5B

func ControlLeftReleased()
{
  if (Control2Contents("ControlLeftReleased")) return(1);
  return(_inherited());
}

func ControlRightReleased()
{
  if (Control2Contents("ControlRightReleased")) return(1);
  return(_inherited());
}
