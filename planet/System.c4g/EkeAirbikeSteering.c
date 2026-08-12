/*-- Eke Reloaded: hold-to-steer airbike handling --*/

// clonk-rs divergence from LegacyClonk. The shipped airbike
// (EkeReloaded.c4d/Weapons.c4d/Airbike.c4d/Script.c) is a latched, single-axis
// vehicle flown on a double integrator with no damping, and this append
// changes three things about that. None of them is a port defect: the engine
// halves all match the oracle and are pinned separately.
//
// 1. It coasts forever. DFA_FLOAT adds FloatAccel = FIXED100(10) = 0.1
//    px/frame^2 for the named ComDir and clamps each axis to
//    FIXED100(Float) = 2.0 px/frame, and COMD_Stop has no deceleration case at
//    all (oracle C4Object.cpp:5291-5309). Nothing in the shipped script ever
//    writes COMD_Stop either, so a tap of [Left] commits the bike to that
//    heading until the pilot spends an exactly equal number of frames holding
//    [Right]. Aiming at a landing spot means integrating by eye.
// 2. It cannot fly a diagonal. ControlLeft/Right/Up/Down each slam ComDir to
//    one pure axis (Script.c:21-90), so the four diagonal ComDirs the engine
//    offers are unreachable and every diagonal has to be flown as a staircase.
// 3. Steering cancels the gun. ControlLeft/Right reset any non-Fly action with
//    `SetAction("Fly", clonk)` (Script.c:23,46) to get the pilot out of
//    Hyperfly, but the same line drops the Shoot action, so turning ends the
//    burst that ControlThrow started (Script.c:92-105,371-395).
//
// This append replaces the steering policy only, for a human pilot only:
//
// - The four steering controls maintain the set of keys the pilot is holding
//   instead of latching one axis, and resolve it exactly the way the engine
//   resolves pressed coms (Coms2ComDir, C4ObjectCom.cpp:903-920). Releasing a
//   key stops that axis' acceleration, holding two flies the diagonal between
//   them, and rolling one direction onto its opposite and letting go of the
//   newer one leaves the older one steering.
// - A per-frame effect decays each unheld axis toward zero by one FloatAccel
//   step, snapping inside the last step. That is deliberately the same
//   magnitude the engine accelerates with, so the bike brakes exactly as fast
//   as it accelerates -- the symmetry DFA_WALK already has in C++
//   (`if ((xdir > -WalkAccel) && (xdir < +WalkAccel)) xdir = 0;`,
//   C4Object.cpp:4796). Terminal speed and the Float physical still belong to
//   the engine; this only removes momentum the pilot is no longer asking for.
// - Only the Fly-family actions are reset by a turn, so a burst survives
//   steering. The Hyperfly boost keeps its shipped shape -- a double tap
//   raises the Float physical to 800 and a turn ends it -- and simply becomes
//   hold-to-dash like everything else, so the four-times-higher bound is only
//   spent while the pilot is asking for it.
//
// Hold-to-steer needs releases, and only a human pilot produces them. The GPED
// remote control reaches the bike through these very handlers -- it is itself
// the object in the AirbikeFly action (Script.c:185), and
// `target -> ControlLeft(this())` lands on the replaced ControlLeft
// (GPED.c4d/Script.c:15-73) -- but no definition in content/ declares a
// Control*Released at all, so a held direction would latch write-only there.
// Every control and the glide therefore test `GetID(clonk) == SF5B` and fall
// through to the shipped pure-axis body otherwise.
//
// Deliberately NOT changed: FloatAccel, the Float physical and its Hyperfly
// ladder, the ActMap, the weapon modes, the dismount rule, Hit/Damage, and the
// whole GPED remote-control path (see also EkeGpedRemoteControl.c).
//
// One consequence worth naming: Hit() explodes the bike above
// `Abs(GetXDir()) >= 70`, i.e. 7.0 px/frame (Script.c:481-486), which only the
// Hyperfly bound of 8.0 can reach. That threshold is untouched, but the ram is
// now a maneuver you have to hold the dash through rather than one a released
// key coasts into.
//
// Determinism contract: no Random() draw is added or reordered, and every
// write is to synchronized object state from synchronized control input, so
// two clonk-rs peers stay in lockstep. It does diverge from a LegacyClonk peer
// and from replays recorded against one.

#strict

// `nowarn`: AB5B ships with EkeReloaded, so every other scenario links this
// script with the target absent (C4AulLink.cpp:42-49).
#appendto AB5B nowarn

// The steering keys the pilot is holding, and the ComDir they resolve to.
//
// The key set has to be the source of truth: with only a composed ComDir there
// is nowhere to record that a second key on the same axis is still down, so
// rolling [Left] onto [Right] and letting go of [Right] would brake the bike
// while the pilot is still holding a direction. Jump'n'Run would paper over
// that on the next `ControlUpdate`; classic control never gets one.
local abHeld;
local abSteer;

private func ABH_Left()  { return(1); }
private func ABH_Right() { return(2); }
private func ABH_Up()    { return(4); }
private func ABH_Down()  { return(8); }

/* Held-direction bookkeeping */

private func AirbikeSteerX() { return(ComDirXSign(abSteer)); }
private func AirbikeSteerY() { return(ComDirYSign(abSteer)); }

// Resolve the held keys exactly the way the engine resolves pressed coms, so
// classic control and Jump'n'Run agree on every combination: only the eight
// exact direction-bit pairs map, and everything else -- including two opposite
// keys held at once -- is COMD_Stop (oracle Coms2ComDir,
// C4ObjectCom.cpp:903-920).
private func AirbikeHeldComDir()
{
  // Parenthesized deliberately: `==` binds tighter than `|`, so an unbraced
  // `abHeld == ABH_Up() | ABH_Right()` reads as `(abHeld == ABH_Up()) |
  // ABH_Right()`, which is always true.
  if (abHeld == ABH_Up())                   return(COMD_Up());
  if (abHeld == (ABH_Up()   | ABH_Right())) return(COMD_UpRight());
  if (abHeld == ABH_Right())                return(COMD_Right());
  if (abHeld == (ABH_Down() | ABH_Right())) return(COMD_DownRight());
  if (abHeld == ABH_Down())                 return(COMD_Down());
  if (abHeld == (ABH_Down() | ABH_Left()))  return(COMD_DownLeft());
  if (abHeld == ABH_Left())                 return(COMD_Left());
  if (abHeld == (ABH_Up()   | ABH_Left()))  return(COMD_UpLeft());
  return(COMD_Stop());
}

private func AirbikeHeldFromComDir(int comdir)
{
  var held = 0;
  if (ComDirXSign(comdir) < 0) held = held | ABH_Left();
  if (ComDirXSign(comdir) > 0) held = held | ABH_Right();
  if (ComDirYSign(comdir) < 0) held = held | ABH_Up();
  if (ComDirYSign(comdir) > 0) held = held | ABH_Down();
  return(held);
}

// Hold-to-steer needs releases, and only a human pilot ever produces them. A
// GPED steers the bike through these same controls
// (`target -> ControlLeft(this())`, GPED.c4d/Script.c:15-73) but has no
// release counterpart anywhere in the content tree, so a held-direction model
// would latch write-only and the remote pilot could never null an axis again.
// Every control therefore falls straight through to the shipped pure-axis
// body for a non-SF5B controller.
private func AirbikeHeldSteering(object clonk) { return(GetID(clonk) == SF5B); }

private func AirbikeSteerApply(object clonk)
{
  abSteer = AirbikeHeldComDir();
  SetComDir(abSteer);
  // Letting go of everything ends the dash too, so the Float physical drops
  // back down the Flying() ladder instead of staying latched at 800 -- with
  // its looping sound and afterburner particles -- on a bike the brake has
  // already stopped (Airbike.c4d/Script.c:296-306,316-340).
  if (abSteer == COMD_Stop() && GetAction() eq "Hyperfly") SetAction("Fly", clonk);
  return(1);
}

private func AirbikeSteerPress(object clonk, int key)
{
  abHeld = abHeld | key;
  return(AirbikeSteerApply(clonk));
}

private func AirbikeSteerRelease(object clonk, int key)
{
  if (!(abHeld & key)) return(0);
  abHeld = abHeld ^ key;
  return(AirbikeSteerApply(clonk));
}

private func AirbikeSteerClear()
{
  abHeld = 0;
  abSteer = COMD_Stop();
  return(1);
}

/* Controls */

// Turning resets the Hyperfly dash and the reload/recoil waits exactly as the
// shipped script does, but leaves Shoot alone so a burst survives steering.
private func AirbikeTurnAction(object clonk)
{
  if (GetAction() eq "Fly")   return(0);
  if (GetAction() eq "Shoot") return(0);
  SetAction("Fly", clonk);
  return(1);
}

func ControlLeft(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(_inherited(clonk));
  AirbikeTurnAction(clonk);
  SetDir(DIR_Left());
  SetDir(DIR_Left(), clonk);
  return(AirbikeSteerPress(clonk, ABH_Left()));
}

func ControlRight(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(_inherited(clonk));
  AirbikeTurnAction(clonk);
  SetDir(DIR_Right());
  SetDir(DIR_Right(), clonk);
  return(AirbikeSteerPress(clonk, ABH_Right()));
}

// The double taps are a separate com (`COM_Left | COM_Double`), so the single
// handler above never sees the press that starts the boost
// (oracle C4Player.cpp:1532-1533). Register the held direction here too, or
// the dash would run with COMD_Stop and never move.
func ControlLeftDouble(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (AirbikeHeldSteering(clonk)) AirbikeSteerPress(clonk, ABH_Left());
  return(_inherited(clonk));
}

func ControlRightDouble(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (AirbikeHeldSteering(clonk)) AirbikeSteerPress(clonk, ABH_Right());
  return(_inherited(clonk));
}

func ControlUp(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(_inherited(clonk));
  return(AirbikeSteerPress(clonk, ABH_Up()));
}

func ControlDown(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);

  // Absteigen -- the shipped dismount rule, unchanged (Script.c:74-90). It has
  // to run for a remote controller too, so it precedes the fall-through.
  if (Stuck() || GBackSolid(0, 11))
  {
    clonk -> SetAction("Walk");
    SetPhysical("Float", 0, 2);
    AirbikeSteerClear();
    return(1);
  }
  if (!AirbikeHeldSteering(clonk)) return(_inherited(clonk));
  return(AirbikeSteerPress(clonk, ABH_Down()));
}

// A release only ever drops the key it owns, so rolling one direction onto
// its opposite and letting go of the newer one leaves the older one steering.
func ControlLeftReleased(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(0);
  return(AirbikeSteerRelease(clonk, ABH_Left()));
}

func ControlRightReleased(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(0);
  return(AirbikeSteerRelease(clonk, ABH_Right()));
}

func ControlUpReleased(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(0);
  return(AirbikeSteerRelease(clonk, ABH_Up()));
}

func ControlDownReleased(object clonk)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(0);
  return(AirbikeSteerRelease(clonk, ABH_Down()));
}

// Jump'n'Run re-sync. `C4Object::CallControl` hands the crew member the live
// `Coms2ComDir(PressedComs)` after every com it dispatches, but only for
// AutoStopControl players (oracle C4Object.cpp:3321-3339). Where it is
// available it is authoritative, so a release lost to a modal or a focus
// change cannot leave the bike accelerating on a key nobody is holding.
func ControlUpdate(object clonk, int comdir)
{
  if (GetAction(clonk) ne "AirbikeFly") return(0);
  if (!AirbikeHeldSteering(clonk))      return(0);
  abHeld = AirbikeHeldFromComDir(comdir);
  abSteer = comdir;
  SetComDir(comdir);
  return(1);
}

// A fresh pilot inherits no held keys. Only a human one: a GPED stays on the
// shipped latched model, where a cleared held direction would be a lie.
func ControlRequest(object requester)
{
  var accepted = _inherited(requester);
  if (accepted && AirbikeHeldSteering(requester)) AirbikeSteerClear();
  return(accepted);
}

/* Glide -- the brake the engine's DFA_FLOAT has no case for */

// Installed from the Fly StartCall rather than Initialize, so airbikes that
// arrive as saved objects -- which are loaded and never Initialize'd -- are
// covered too.
func Flying()
{
  if (!GetEffect("AirbikeGlide", this())) AddEffect("AirbikeGlide", this(), 1, 1, this());
  return(_inherited());
}

// FloatAccel = FIXED100(10), i.e. 10 hundredths of a pixel per frame.
private func AirbikeGlideStep() { return(10); }

private func AirbikeBrake(int vel, int step)
{
  if (vel >  step) return(vel - step);
  if (vel < -step) return(vel + step);
  return(0);
}

public func FxAirbikeGlideTimer(object target, int fxnum, int time)
{
  // An abandoned airbike keeps the shipped physics: PilotLost parks it on
  // COMD_Down and it sinks (Script.c:430-438).
  //
  // A GPED sits in the same AirbikeFly action and is the same action target
  // while it remote-controls the bike, and it steers by calling these very
  // controls (`target -> ControlLeft(this())`, GPED.c4d/Script.c:15-73), so
  // the held-direction bookkeeping already covers it. It has no release
  // counterpart though, so a remote pilot could never let go and the brake
  // would be unreachable anyway; keep the remote-control path on the shipped
  // physics outright, which is the same SF5B test the shipped ControlRequest
  // makes (Script.c:186).
  var clonk = GetActionTarget();
  if (GetID(clonk) != SF5B)             return(1);
  if (GetAction(clonk) ne "AirbikeFly") return(1);

  // Cooperate with every other writer instead of overruling them. A ComDir the
  // append did not ask for means a script commanded the bike directly -- the
  // scenario's opening `SetComDir(COMD_Up(), airbike)`
  // (AirbikeFight.c4s/Script.c:59), or `PilotLost`'s `SetComDir(COMD_Down())`
  // still standing at the moment a fresh pilot mounts (Script.c:430-438) --
  // and braking that at exactly the FloatAccel the engine adds would cancel it
  // invisibly, leaving the bike creeping on an axis nobody is steering.
  // Adopting it instead honours the command and hands the pilot the wheel on
  // the first key.
  if (abSteer == COMD_Stop() && GetComDir() != COMD_Stop()) abSteer = GetComDir();

  var step = AirbikeGlideStep();
  if (!AirbikeSteerX()) SetXDir(AirbikeBrake(GetXDir(0, 100), step), 0, 100);
  if (!AirbikeSteerY()) SetYDir(AirbikeBrake(GetYDir(0, 100), step), 0, 100);
  return(1);
}

/* ComDir <-> axis signs */

global func ComDirXSign(int comdir)
{
  if (comdir == COMD_Left()  || comdir == COMD_UpLeft()  || comdir == COMD_DownLeft())  return(-1);
  if (comdir == COMD_Right() || comdir == COMD_UpRight() || comdir == COMD_DownRight()) return(+1);
  return(0);
}

global func ComDirYSign(int comdir)
{
  if (comdir == COMD_Up()   || comdir == COMD_UpLeft()   || comdir == COMD_UpRight())   return(-1);
  if (comdir == COMD_Down() || comdir == COMD_DownLeft() || comdir == COMD_DownRight()) return(+1);
  return(0);
}

global func ComDirFromSigns(int x, int y)
{
  if (y < 0)
  {
    if (x < 0) return(COMD_UpLeft());
    if (x > 0) return(COMD_UpRight());
    return(COMD_Up());
  }
  if (y > 0)
  {
    if (x < 0) return(COMD_DownLeft());
    if (x > 0) return(COMD_DownRight());
    return(COMD_Down());
  }
  if (x < 0) return(COMD_Left());
  if (x > 0) return(COMD_Right());
  return(COMD_Stop());
}
