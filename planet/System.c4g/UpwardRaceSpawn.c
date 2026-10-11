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
  // Recognise the race by its course, which every peer scripts alike.
  // Head.Title differs per peer: a network client's core carries the
  // host's localized title (C4GameSave.cpp:79; C4Network2.cpp:619-667).
  if (GameCall("GetRACEDirection") != 3
      || GameCall("GetRACEStartOffset") != 90
      || GameCall("GetRACEEndOffset") != 140
      // Abwaerts' own Recruitment() inherits without the player, so the
      // broadcast always names player 0; the owner is the real one.
      || FindObjectOwner(SGNL, GetOwner(clonk)))
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
