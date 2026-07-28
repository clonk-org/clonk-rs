/*-- Eke Reloaded: hold-to-steer guided missiles --*/

// clonk-rs divergence from LegacyClonk. The shipped rocket launcher latches
// the remote-guidance turn into the missile's "command" local and only
// [Down]/[Up] clear it again (EkeReloaded.c4d/Weapons.c4d/RocketLauncher.c4d/
// Script.c:9-49), so a tapped turn key spins the missile until the player
// straightens it by hand. These appends stop the turn when the steering key
// goes up, so the missile turns only while a turn key is held.
//
// A release only clears the direction it owns: rolling from [Left] onto
// [Right] keeps the newer turn when the stale key comes up.

#strict

#appendto RL5B

func ControlLeftReleased()  { return(StopGuidedTurn("Left")); }
func ControlRightReleased() { return(StopGuidedTurn("Right")); }

func StopGuidedTurn(string direction)
{
  if (!LocalN("guiding")) return(0);

  var guided = LocalN("missile");
  if (!guided) return(0);

  // A newer turn already took over; its own release will clear it.
  if (LocalN("command", guided) ne direction) return(1);

  LocalN("command", guided) = "Straight";
  return(1);
}
