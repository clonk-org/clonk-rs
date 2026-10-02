/*-- A safe foothold when joining Team Downhill Race upwards --*/

// Intentional content change for clonk-org/clonk-rs#1826. The scenario's
// JoinPlayer places both new and relaunched Clonks at the start (or their
// checkpoint), even after explosions have removed the ground beneath it.
// Keep this policy in script; the engine's terrain and movement stay unchanged.

#strict 2
#appendto RACE nowarn

public func OnClonkRecruitment(object clonk, int player)
{
  var result = _inherited(clonk, player);
  // Both Abwaerts races share this internal title; only Falschrum goes up.
  if (GetScenarioVal("Title", "Head") != "RunterRennenTeam"
      || GameCall("GetRACEDirection") != 3)
    return result;

  // Recruitment runs before JoinPlayer sets the final spawn position.
  ScheduleCall(this(), "ClonkRsUpwardRaceSpawnBridge", 1, 1, clonk);
  return result;
}

private func ClonkRsUpwardRaceSpawnBridge(object clonk)
{
  if (!clonk || !GetAlive(clonk))
    return;
  // The intact start is a few pixels below the initial feet position.
  for (var dy = 10; dy < 15; ++dy)
    if (clonk->GBackSolid(0, dy))
      return;
  // ACLK's bottom contact vertex is y=9. Leave the body clear and put a
  // short bridge one pixel below it, including at a checkpoint respawn.
  var x = GetX(clonk), y = GetY(clonk) + 10;
  // LOAM::BridgeMaterial uses Earth (Objects.c4d/Items.c4d/Materials.c4d/Loam.c4d/Script.c:15-18).
  DrawMaterialQuad("Earth", x-20,y, x+20,y, x+20,y+4, x-20,y+4);
}
