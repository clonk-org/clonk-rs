/*-- Eke Reloaded: keep the pilot parked while the GPED steers an airbike --*/

// clonk-rs divergence from LegacyClonk. Jump'n'Run control - the default a new
// player is created with (C4StartupPlrSelDlg.cpp:1103-1113) - refreshes the
// crew member's ComDir from the pressed keys for every com its script does not
// consume (C4Object::AutoStopDirectCom's `default: AutoStopUpdateComDir()`,
// C4Object.cpp:3579-3593,3743-3755). C4Player::InCom and
// C4Player::ExecuteControl emit a stale `LastCom | COM_Single` com whenever the
// previous key is superseded or its double-click window expires
// (C4Player.cpp:1212-1228,1522-1531), and nothing in the Eke SF5B -> GP5B chain
// answers ControlLeftSingle/RightSingle/UpSingle/DownSingle - or the
// "Undefined" name ComName gives a LastCom that already carries COM_Double
// (C4ObjectCom.cpp:800-851). The GPED consumes all four steering coms while it
// remote-controls an airbike (GPED.c4d/Script.c:15-73), so those stale coms are
// the only thing left that walks the pilot off the spot GPED::ControlDig parked
// him on - clonk-org/clonk-rs#202.

#strict

// `nowarn`: the SFT ships with EkeReloaded, so every other scenario links this
// script with the target absent (C4AulLink.cpp:42-49).
#appendto SF5B nowarn

func ControlLeftSingle()     { if (GpedSteersAirbike()) return(1); return(_inherited()); }
func ControlRightSingle()    { if (GpedSteersAirbike()) return(1); return(_inherited()); }
func ControlUpSingle()       { if (GpedSteersAirbike()) return(1); return(_inherited()); }
func ControlDownSingle()     { if (GpedSteersAirbike()) return(1); return(_inherited()); }
func ControlThrowSingle()    { if (GpedSteersAirbike()) return(1); return(_inherited()); }
func ControlSpecialSingle()  { if (GpedSteersAirbike()) return(1); return(_inherited()); }
func ControlSpecial2Single() { if (GpedSteersAirbike()) return(1); return(_inherited()); }
func ControlUndefined()      { if (GpedSteersAirbike()) return(1); return(_inherited()); }

// Only the selected item receives the SFT's forwarded controls
// (SFT.c4d/Script.c:288-295), and the GPED holds them only once
// Airbike::ControlRequest has put it in its AirbikeFly action and left a target
// behind (Airbike.c4d/Script.c:180-199).
private func GpedSteersAirbike()
{
  var gped = Contents();

  if (GetID(gped) != GP5B)             return(0);
  if (GetAction(gped) ne "AirbikeFly") return(0);
  if (!LocalN("target", gped))         return(0);

  return(1);
}
