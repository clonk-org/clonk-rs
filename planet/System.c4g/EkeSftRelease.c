/*-- Eke Reloaded: forward hold-to-steer pilot controls --*/

// clonk-rs divergence from LegacyClonk. The Eke SFT forwards ControlLeft and
// ControlRight to its selected item and to the airbike it is sitting on
// (EkeReloaded.c4d/Creatures.c4d/SFT.c4d/Script.c:40-104,279-295) but has no
// release counterpart, so neither can learn that the key went up. These
// appends complete the pair; every target without a *Released callback is
// unaffected, because ObjectCall on a missing function is a no-op. They also
// keep a quick Down release/repress from losing the airbike's dismount check
// when C4Player promotes that landing press to ControlDownDouble.
//
// The airbike is asked first and the selected item second, matching the order
// the press handlers already use. See EkeAirbikeSteering.c for what the
// airbike does with them.

#strict

// `nowarn`: the SFT ships with EkeReloaded, so every other scenario links
// this script with the target absent (C4AulLink.cpp:42-49).
#appendto SF5B nowarn

func ControlLeftReleased()
{
  if (Control2Airbike("ControlLeftReleased"))  return(1);
  if (Control2Contents("ControlLeftReleased")) return(1);
  return(_inherited());
}

func ControlRightReleased()
{
  if (Control2Airbike("ControlRightReleased"))  return(1);
  if (Control2Contents("ControlRightReleased")) return(1);
  return(_inherited());
}

func ControlUpReleased()
{
  if (Control2Airbike("ControlUpReleased"))  return(1);
  if (Control2Contents("ControlUpReleased")) return(1);
  return(_inherited());
}

func ControlDownReleased()
{
  if (Control2Airbike("ControlDownReleased"))  return(1);
  if (Control2Contents("ControlDownReleased")) return(1);
  return(_inherited());
}

// A landing press can still be a double-click: releasing does not clear
// C4Player::LastCom and its ten-frame timeout is still running (oracle
// C4Player.cpp:1213-1228,1522-1548; C4Constants.h:156), while the shipped SFT's
// ControlDownDouble deliberately forwards nothing
// (EkeReloaded.c4d/Creatures.c4d/SFT.c4d/Script.c:145-151). Preserve the
// ordinary Down meaning while seated so the airbike can run its grounded
// dismount check; off the bike, keep the shipped double-click behavior.
func ControlDownDouble()
{
  if (Control2Airbike("ControlDown")) return(1);
  return(_inherited());
}

// `C4Object::CallControl` hands the crew member the live
// `Coms2ComDir(PressedComs)` after every com it dispatches, but only for
// AutoStopControl players (oracle C4Object.cpp:3321-3339). Forward it to the
// airbike, which uses it to re-sync its held-direction state; Control2Airbike
// cannot carry the second parameter.
func ControlUpdate(object byObject, int comdir, bool dig, bool throw, bool special, bool special2)
{
  if (GetAction() eq "AirbikeFly")
    ObjectCall(GetActionTarget(), "ControlUpdate", this(), comdir);
  return(_inherited(byObject, comdir, dig, throw, special, special2));
}
