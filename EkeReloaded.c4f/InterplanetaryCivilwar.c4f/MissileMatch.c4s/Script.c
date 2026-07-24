/* MissileMatch */

#strict

func Initialize()
{
  var c1 = RGB(010, 009, 010);
  var c2 = RGB(161, 157, 159);
  var c3 = RGB(255, 254, 255);

  SetGamma(c1, c2, c3);

  ScriptGo(1);
  return(1);
}

func Script1()
{
  SetMaxPlayer();
  return(1);
}

protected func InitializePlayer(player)
{ 
  var i = GetCrewCount(player);
  while (i--)
  {
    var clonk = GetCrew(player, i);

    var jetpack = CreateContents(JP5B, clonk);
    LocalN("ammo", jetpack) = 100;

    CreateContents(TB5B, clonk);
    CreateContents(FLAG, clonk);
    CreateContents(NH5B, clonk);
  }
  return(1);
}