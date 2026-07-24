/* Equiem */

#strict

static uziBird;

func GetSftSuit() { return("Urban"); }

func Initialize()
{
  CreateUziBird();
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
  // es sollen genug Zaps im Graben #2 leben
  var zapCount = ObjectCount(ZAP1, 150, 200, 90, 200);
  if (zapCount < 10) CreateObject(ZAP1, RandomX(150, 240), RandomX(200, 400), NO_OWNER);

  // Neohexin regnen lassen
  var random1 = !Random(25 - Min(GetPlayerCount() * 2, 25));
  if (random1) CreateObject(NH5B, Random(LandscapeWidth()), 0, NO_OWNER);

  // Munition regnen lassen
  var random2 = !Random(55 - Min(GetPlayerCount() * 2, 55));
  if (random2) CreateObject(CA5B, Random(LandscapeWidth()), 0, NO_OWNER);

  // Benzin am Spawnpoint nachliefern
  if (!FindObject(GS5B, 500, 393)) CreateObject(GS5B, 500, 401, NO_OWNER);

  // ist unser UziBird noch am Leben?
  if (!GetAlive(uziBird)) CreateUziBird();
 
  goto(2);
  return(1);
}

private func CreateUziBird()
{
  uziBird = CreateObject(BIRD, Random(LandscapeWidth()), 25, NO_OWNER);
  var uzi = CreateContents(UZ5B, uziBird);

  LocalN("ammo", uzi) = 100;
  return(1);
}

private func InitializeClonk(clonk)
{ 
  SetPosition(Random(LandscapeWidth()), Random(100) + 25, clonk);   

  var jetpack = CreateContents(JP5B, clonk);
  LocalN("ammo", jetpack) = 100;

  CreateContents(GS5B, clonk);
  CreateContents(PM5B, clonk);

  clonk -> SetAction("JetpackFly");
  return(1);
}