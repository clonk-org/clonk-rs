/* HarpoonRace */

#strict

func GetSftSuit() { return("Storm"); }

func Initialize()
{
  Log("Go!");
  ScriptGo(1);
  return(1);
}

func Script1()
{
  SetMaxPlayer();
  return(1);
}

func Script6()
{
  // Ausrüstung regnen lassen
  if (!Random(10)) CreateObject(HG5B, Random(LandscapeWidth()));
  if (!Random(20)) CreateObject(PM5B, Random(LandscapeWidth()));

  if (!Random(50))
  {
    var uzi = CreateObject(UZ5B, Random(LandscapeWidth()));
    LocalN("ammo", uzi) = 100;
  }
  if (!Random(80))
  {
    var flamethrower = CreateObject(FT5B, Random(LandscapeWidth()));
    LocalN("ammo", flamethrower) = 100;
  }
  goto(2);
  return(1);
}

func GetRACEDirection()   { return(001); }
func GetRACEStartOffset() { return(180); }
func GetRACEEndOffset()   { return(100); }

protected func InitializePlayer(player)
{ 
  var clonk = GetCrew(player);
  InitializeClonk(clonk);
  return(1);
}

private func InitializeClonk(clonk)
{ 
  SetPosition(10 + Random(100), LandscapeHeight() / 2 - 15, clonk);

  CreateContents(HP5B, clonk);
  CreateContents(NH5B, clonk);

  clonk -> SetAction("HarpoonWalk");
  return(1);
}

protected func RelaunchPlayer(player, lastSft)
{
  Log("%s tried to be Spiderman.", GetPlayerName(player));

  var clonk = CreateObject(SF5B, 0, 0, player);
  
  if (lastSft)
  {
    // den gerade gestorbenen SFT nochmal antreten lassen
    GrabObjectInfo(lastSft, clonk);
  }
  else MakeCrewMember(clonk, player);
  
  clonk -> DoEnergy(100);
  SelectCrew(player, clonk, 1);

  InitializeClonk(clonk);
  return(1);
}                 