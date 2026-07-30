/*-- Continuous bird flight steering --*/

// clonk-rs divergence from LegacyClonk. The shipped bird
// (Objects.c4d/Animals.c4d/Bird.c4d/Script.c:75-90) steers with four
// independent coin flips per Activity tick, and Activity runs on the default
// 35-frame TimerCall (C4Def.cpp:298). Every decision slams ComDir to a pure
// axis -- COMD_Up/COMD_Down for climb, COMD_Left/COMD_Right for turns -- so
// the bird never once uses the four diagonal ComDirs the engine offers.
//
// The actuation model it drives is a double integrator with no damping:
// DFA_FLOAT adds FLOAT_ACCEL = FIXED100(10) = 0.1 px/frame^2 per named axis
// and clamps each axis to FIXED100(Float) = 2.0 px/frame, and COMD_Stop has
// no deceleration case at all (C4Object.cpp:5268-5286). 21 frames to reach
// terminal, 41 to reverse an axis -- against a 35-frame decision period. The
// bird is permanently mid-transient, which is what makes it read as an insect
// rather than a bird.
//
// This append replaces the steering policy only. It keeps a heading and flies
// it with per-frame sub-pixel velocity writes, so turns are flown rather than
// snapped. Every write stays inside +-200 at precision 100, which is below the
// engine clamp, so this controller is the sole velocity authority while it is
// engaged and the engine adds nothing on top.
//
// Determinism contract: this file adds NO synchronized Random() draw. The
// shipped Activity/ContactLeft/ContactRight draw sequences are reproduced call
// for call, in order, with the same arguments and the same short-circuits, so
// RandomCount is unchanged frame for frame; the new variation comes from
// ObjectNumber(), a synced per-object integer that survives save/load, and
// from FrameCounter(). Coherent wander wants a smooth low-frequency signal
// anyway, which is what two incommensurate sine terms give and what an
// independent uniform draw per tick cannot.
//
// Deliberately NOT changed: the possession block, the bait/nest/reproduction
// logic, ActMap.txt and DefCore.txt. Every shipped AI entry point keeps its
// GetEffect("PossessionSpell") guard, and this controller stands down for
// possessed, contained and commanded birds so C4Command and the possession
// spell keep exclusive ownership of ComDir when they hold it.

#strict

// `nowarn`: BIRD ships in Objects.c4d, which every scenario loads, but engine
// fixtures link System.c4g against a definition set that has no bird in it
// (C4AulLink.cpp:42-49).
#appendto BIRD nowarn

local flight_heading;   // 0..359 in Clonk orientation: 0 = up, 90 = right
local flight_cruise;    // target speed, hundredths of a pixel per frame
local flight_agility;   // per-frame velocity step clamp, hundredths
local flight_urgency;   // extra agility while escaping, counts itself down
local flight_alarm;     // startle level 0..100, rises at once and decays slowly

private func FlightThinkInterval() { return(8); }

/* Installation */

// Installed lazily rather than from Initialize: birds also arrive from
// Objects.txt as saved objects, which are loaded and never Initialize'd, and
// keeping out of Initialize means no draw is added at scenario-init time so
// the placement ledger of every bird-bearing scenario is untouched.
private func FlightEnsure()
{
  if (GetEffect("BirdFlight", this())) return(0);
  return(AddEffect("BirdFlight", this(), 1, 1, this()));
}

public func FxBirdFlightStart(object target, int fxnum, int temp)
{
  if (temp) return(1);
  // Seed from whatever the shipped Initialize/Birth already picked, so the
  // one Random(2) they spend still decides which way this bird sets off.
  flight_heading = 90;
  if (GetXDir(0, 100) < 0 || GetComDir() == COMD_Left()) flight_heading = 270;
  flight_cruise  = 120;
  flight_agility = 2;
  flight_urgency = 0;
  flight_alarm   = 0;
  return(1);
}

public func FxBirdFlightTimer(object target, int fxnum, int time)
{
  // Think on a stagger so a flock does not sample the world in lockstep.
  if (!((FrameCounter() + ObjectNumber()) % FlightThinkInterval())) FlightThink();
  FlightStep();
  return(1);
}

/* Actuator -- every frame */

private func FlightStep()
{
  // Possession, containers and commands own ComDir outright while they hold
  // it; a Follow/MoveTo command steers a floater itself every frame.
  if (GetEffect("PossessionSpell", this())) return(0);
  if (Contained() || GetCommand()) return(0);
  // Only free flight is ours. BuildNest zeroes both dirs on purpose, and
  // Eat/Attack/Tumble are left exactly as the shipped script drives them.
  if (GetAction() ne "Fly" && GetAction() ne "Turn") return(0);
  // Not seeded yet (a save from before this append): leave ComDir physics be.
  if (!flight_agility) return(0);

  var want_x =  Sin(flight_heading, flight_cruise, 1);
  var want_y = -Cos(flight_heading, flight_cruise, 1);
  var step   = flight_agility + flight_urgency;
  var vx     = GetXDir(0, 100);
  var vy     = GetYDir(0, 100);

  SetXDir(vx + BoundBy(want_x - vx, -step, step), 0, 100);
  SetYDir(vy + BoundBy(want_y - vy, -step, step), 0, 100);
  SetComDir(COMD_Stop);

  if (flight_urgency) flight_urgency--;

  // Facing gets a wide deadband: SetDir fires the 20-frame Turn action
  // (ActMap TurnAction=Turn), so a strobing sign would stall the wing cycle.
  if (GetAction() eq "Fly")
  {
    if (vx >  45 && GetDir() == DIR_Left())  SetDir(DIR_Right());
    if (vx < -45 && GetDir() == DIR_Right()) SetDir(DIR_Left());
  }
  return(1);
}

/* Planner -- every FlightThinkInterval() frames */

private func FlightThink()
{
  if (GetEffect("PossessionSpell", this())) return(0);
  if (Contained() || GetCommand()) return(0);

  flight_cruise  = FlightGlideSpeed();
  flight_agility = 2;

  // Prioritised allocation rather than a weighted sum of steering forces: a
  // sum lets avoidance and pursuit cancel, which is the classic way a flyer
  // ends up grinding along the surface it is trying to clear.
  if (FlightAvoidTerrain()) return(1);
  if (FlightFlee()) return(1);
  FlightSeparate();
  FlightWander();
  return(1);
}

// Flap-gliding is a speed cycle, not a height cycle: birds beat in bursts and
// coast between them. 40 frames is a touch over a second at 36 fps.
private func FlightGlideSpeed()
{
  if (((FrameCounter() + ObjectNumber() * 13) % 40) < 16) return(150);
  return(95);
}

private func FlightPathClear(int ang, int len)
{
  return(PathFree(GetX(), GetY(), GetX() + Sin(ang, len, 1), GetY() - Cos(ang, len, 1)));
}

// Three feelers, look-ahead scaled by speed. The shipped script probes for
// liquid and fire but never once calls GBackSolid or PathFree, so terrain is
// handled purely on impact.
private func FlightAvoidTerrain()
{
  var len = 40 + (Abs(GetXDir(0, 100)) + Abs(GetYDir(0, 100))) / 5;
  if (!FlightPathClear(flight_heading, len))
  {
    var left  = FlightPathClear((flight_heading + 300) % 360, len * 2 / 3);
    var right = FlightPathClear((flight_heading +  60) % 360, len * 2 / 3);
    if (left && !right)      flight_heading = (flight_heading + 300) % 360;
    else if (right && !left) flight_heading = (flight_heading +  60) % 360;
    // Both open, or boxed in: split the tie on ObjectNumber so two birds in
    // the same corner peel apart instead of mirroring each other forever.
    else if (left && right)  flight_heading = (flight_heading + 360 + 60 - 120 * (ObjectNumber() % 2)) % 360;
    else                     flight_heading = (flight_heading + 180) % 360;
    flight_urgency = 6;
    return(1);
  }
  // Keep air underneath and stay off the map ceiling.
  if (GetY() < 60)            flight_heading = FlightTurnToward(flight_heading, 180, 6);
  else if (GBackSolid(0, 40)) flight_heading = FlightTurnToward(flight_heading,   0, 6);
  return(0);
}

private func FlightFlee()
{
  // Nervousness varies per bird, for free, from ObjectNumber.
  var clonk = FindObject2(Find_OCF(OCF_CrewMember()), Find_OCF(OCF_Alive()),
                          Find_NoContainer(),
                          Find_Distance(60 + 40 * (ObjectNumber() % 3)),
                          Sort_Distance());
  if (clonk && !flight_alarm) flight_alarm = 100;
  if (!flight_alarm) return(0);
  // Alarm rises in one tick and decays over ~400 frames. That asymmetry is
  // what reads as spooked rather than twitchy.
  if (clonk)
    flight_heading = FlightTurnToward(flight_heading,
                                      Angle(GetX(clonk), GetY(clonk), GetX(), GetY()), 25);
  flight_cruise  = 190;
  flight_agility = 6;
  flight_alarm  -= 2;
  return(1);
}

// Separation with weak alignment, and deliberately no cohesion: with air
// placement the birds are already loosely co-located, and cohesion is the term
// that collapses a flock into a single point.
private func FlightSeparate()
{
  var other, n, sx, sy, hx, hy, d;
  var flock = FindObjects(Find_ID(BIRD), Find_Distance(90), Find_Exclude(this()));
  for (other in flock)
  {
    // Drained with `continue` rather than `break`, which the shipped bird
    // never uses at this #strict level. k = 5 bounds the cost in a dense
    // flock, and FindObjects order is main-list order, so which five is
    // deterministic.
    if (n >= 5) continue;
    d = ObjectDistance(other, this());
    if (d < 8) d = 8;
    sx += (GetX() - GetX(other)) * 90 / d;
    sy += (GetY() - GetY(other)) * 90 / d;
    hx += GetXDir(other, 100);
    hy += GetYDir(other, 100);
    n++;
  }
  if (!n) return(0);
  // Blend by angle, not by summing vectors into the heading: integer force
  // sums cancel against each other and deadlock.
  if (sx || sy) flight_heading = FlightTurnToward(flight_heading, Angle(0, 0, sx, sy), 12);
  if (hx || hy) flight_heading = FlightTurnToward(flight_heading, Angle(0, 0, hx, hy),  4);
  return(1);
}

// Wander by randomising the DERIVATIVE of the heading, not the heading. Two
// incommensurate periods, seeded per bird, give a drifting course that never
// quite repeats -- and cost no synchronized draw.
private func FlightWander()
{
  var t = FrameCounter();
  var p = ObjectNumber();
  var drift = Sin((t * 2 + p * 37) % 360, 22, 1) + Sin((t * 5 + p * 71) % 360, 10, 1);
  flight_heading = (flight_heading + 360 + drift / 4) % 360;
  return(1);
}

// Shortest signed turn, clamped. The +540 keeps the modulo off negatives.
private func FlightTurnToward(int from, int to, int max_step)
{
  var delta = (to - from + 540) % 360 - 180;
  return((from + BoundBy(delta, -max_step, max_step) + 360) % 360);
}

/* Shipped AI entry points, resteered */

// Reproduces Bird.c4d/Script.c:25-91 call for call. Every Random() below is
// the shipped draw, in the shipped order, under the shipped condition; only
// what the steering statements DO is different.
protected func Activity()
{
  FlightEnsure();

  if (!Random(25)) Sound("Raven*");

  if (GetXDir() > 0 && GetDir() == DIR_Left)  return(TurnRight());
  if (GetXDir() < 0 && GetDir() == DIR_Right) return(TurnLeft());

  if (GetEffect("PossessionSpell", this())) return(0);
  if (Contained()) return(0);
  if (GetCommand()) return(0);

  if (FindContents(BIRD)) Reproduction();

  if (Bait) {
    if (Contained(Bait)) Bait = 0;
    if (ObjectDistance(Bait, this()) > 300) Bait = 0;
    if (ObjectDistance(Bait, this()) <= 25)
      if (GetAction() eq "Fly")
        SetAction("Eat");
  }

  var pObj, aList;
  if (GetCon() == 100)
    if (!Random(5)) {
      aList = FindObjects(Find_Distance(250), Find_NoContainer(), Find_Func("IsBait"));
      for (pObj in aList) {
        if (!WildcardMatch(GetAction(pObj), "*MeatBait*")) continue;
        if (Random(100) >= pObj->~IsBait()) continue;
        SetCommand(this(), "Follow", pObj);
        Bait = pObj;
      }
    }

  if (Random(2) || GetAction() ne "Fly") return(0);

  // Shipped: SetComDir(COMD_Up), then COMD_Down on the flip -- a hard snap to
  // a pure vertical that throws away all horizontal thrust. Bias the heading
  // toward the same climb or dive instead, and let it be flown.
  if (Random(2)) flight_heading = FlightTurnToward(flight_heading, 180, 18);
  else           flight_heading = FlightTurnToward(flight_heading,   0, 18);

  if (!Random(4)) return(0);

  if (!Random(ReproductionRate()))
    Reproduction();

  if (Random(2)) return(TurnRight());
  return(TurnLeft());
}

// Shipped TurnRight/TurnLeft zeroed the opposing velocity outright and pinned
// ComDir to a pure axis. Steer instead; the actuator owns the facing flip.
public func TurnRight()
{
  if (Stuck() || (GetAction() ne "Fly" && GetAction() ne "Turn")) return();
  flight_heading = FlightTurnToward(flight_heading, 90, 55);
  flight_urgency = 3;
  return(1);
}

public func TurnLeft()
{
  if (Stuck() || (GetAction() ne "Fly" && GetAction() ne "Turn")) return();
  flight_heading = FlightTurnToward(flight_heading, 270, 55);
  flight_urgency = 3;
  return(1);
}

// The shipped ContactRight was a copy of ContactLeft, so its
// COMD_Right + Random(2)*2-1 evaluated to COMD_UpRight or COMD_DownRight
// (COMD_Right == 3) -- both of them back into the wall that had just raised
// the callback. Reflecting off the contact side fixes that structurally.
// The Random(5)/Random(2) pair is kept exactly as shipped, including the
// short-circuit that only spends the second draw when the first lands on 0.
protected func ContactLeft()
{
  if (GetEffect("PossessionSpell", this())) return();
  flight_heading = FlightTurnToward(flight_heading, 90, 120);
  flight_urgency = 8;
  if (!Random(5)) flight_heading = (flight_heading + 360 + 25 - 50 * Random(2)) % 360;
  return(1);
}

protected func ContactRight()
{
  if (GetEffect("PossessionSpell", this())) return();
  flight_heading = FlightTurnToward(flight_heading, 270, 120);
  flight_urgency = 8;
  if (!Random(5)) flight_heading = (flight_heading + 360 + 25 - 50 * Random(2)) % 360;
  return(1);
}

protected func ContactTop()
{
  if (GetEffect("PossessionSpell", this())) return();
  flight_heading = FlightTurnToward(flight_heading, 180, 120);
  flight_urgency = 8;
  return(1);
}

protected func ContactBottom()
{
  if (GetEffect("PossessionSpell", this())) return();
  flight_heading = FlightTurnToward(flight_heading, 0, 120);
  flight_urgency = 8;
  return(1);
}

// PhaseCall on Fly, so this runs every frame the bird is airborne. Shipped
// body snapped ComDir to COMD_Up; steer hard instead, and take the chance to
// install the controller on saved birds that never ran Initialize.
protected func Survive()
{
  FlightEnsure();

  if (GetEffect("PossessionSpell", this())) return();

  if (InLiquid() || GBackLiquid(GetXDir() * 2, GetYDir() * 2)
      || GBackLiquid(GetXDir() * 3, GetYDir() * 3)
      || FindObject(0, -20 + GetXDir() * 2, -20 + GetYDir() * 2, 40, 40, OCF_OnFire()))
  {
    flight_heading = FlightTurnToward(flight_heading, 0, 40);
    flight_urgency = 8;
  }
}
