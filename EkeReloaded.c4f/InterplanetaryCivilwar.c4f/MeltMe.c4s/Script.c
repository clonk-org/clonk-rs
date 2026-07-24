/* MeltMe */

#strict

func GetSftSuit() { return("Snow"); }

func Initialize()
{
  SetGamma(RGB(2, 3, 10), RGB(131, 135, 158), RGB(252, 253, 255));
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
  // Neohexin regnen lassen
  var random = !Random(25 - Min(GetPlayerCount() * 2, 25));
  if (random) CreateObject(NH5B, Random(LandscapeWidth()), 0, NO_OWNER);

  goto(2);
  return(1);
}

private func InitializeClonk(clonk)
{ 
  // Position berechnen
  //********************
  var xPos = Random(LandscapeWidth());
  var yPos = Random(25) + 50;

  for (var i = 0; FindObject2(Find_ID(SF5B), Find_Distance(200, xPos, yPos)) && i < 25; i++)
  {
    xPos = Random(LandscapeWidth());
    yPos = Random(25) + 50;
  }
  var balloon = CreateObject(BALN, xPos, yPos, GetOwner(clonk));
  SetPosition(GetX(balloon) - RandomX(-5, 5), GetY(balloon) + 13, clonk);

  // Clonk soll Ballon anfassen, dann fällt er nicht so schnell runter
  clonk -> SetAction("Push", balloon);

  // Ballon vermienen
  var blaster = CreateContents(OB5B, balloon);
  blaster -> SetAction("Countdown");
  
  // Clonk ausrüsten
  //*****************
  
  var assaultRifle = CreateContents(AR5B, clonk); 
  LocalN("ammo", assaultRifle) = 100;
  LocalN("qGrenades", assaultRifle) = 100;

  var flamethrower = CreateContents(FT5B, clonk);
  LocalN("ammo", flamethrower) = 100;

  CreateContents(CA5B, clonk);
  CreateContents(GB5B, clonk);
  CreateContents(GS5B, clonk);
  return(1);
}