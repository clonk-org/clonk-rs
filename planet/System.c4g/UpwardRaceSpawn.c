/*-- A safe foothold when joining Team Downhill Race upwards --*/

// Intentional content change for clonk-org/clonk-rs#1826. The scenario's
// JoinPlayer places both new and relaunched Clonks at the start, even after
// explosions have removed the ground beneath it.
// Keep this policy in script; the engine's terrain and movement stay unchanged.

#strict 2
#appendto RACE nowarn

public func OnClonkRecruitment(object clonk, int player)
{
  var result = _inherited(clonk, player);
  // Both Abwaerts races share this internal title; only Falschrum goes up.
  if (GetScenarioVal("Title", "Head") != "RunterRennenTeam"
      || GameCall("GetRACEDirection") != 3
      || FindObjectOwner(SGNL, player))
    return result;

  // Recruitment runs before JoinPlayer sets the final spawn position.
  ScheduleCall(this(), "ClonkRsUpwardRaceSpawnBridge", 1, 1, clonk);
  return result;
}

private func ClonkRsUpwardRaceSpawnBridge(object clonk)
{
  if (!clonk || !GetAlive(clonk))
    return;
  // The authored surface is fixed at y=3710; repeated respawns cannot
  // build above it. Checkpoint spawns retain their authored behavior.
  var bridgeY = 3710;
  if (clonk->GBackSolid(0, bridgeY - GetY(clonk)))
    return;
  var x = GetX(clonk);
  // LOAM::BridgeMaterial uses Earth (Objects.c4d/Items.c4d/Materials.c4d/Loam.c4d/Script.c:15-18).
  DrawMaterialQuad("Earth", x-20,bridgeY, x+20,bridgeY,
                   x+20,bridgeY+4, x-20,bridgeY+4);
}
