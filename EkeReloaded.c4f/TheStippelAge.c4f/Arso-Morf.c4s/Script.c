/* Arso-Morf */

#strict 2

static scientist;

func GetSftSuit() { return("Night"); }

func Initialize()
{
  // Ziele und Regeln erzeugen
  if (!ObjectCount(SE5B)) CreateObject(SE5B);
  if (!ObjectCount(NF5B)) CreateObject(NF5B);

  // Auftritt vom Mad Scientist
  scientist = CreateObject(CLNK, 1316, 680, GetPlayerByName("I.S.I"));
  var key = CreateContents(KC5B, scientist);
  key -> SetCode(0xFFDD00);

  SetCommand(scientist, "MoveTo", 0, LandscapeWidth(), 680);
  
  scientist -> SetName("Mad Scientist");
  scientist -> SetPhysical("Energy", 100000, 2);
  scientist -> DoEnergy(100);
  scientist -> Incinerate();

  ScriptGo(true);
}

func Script20()
{
  if (GetAlive(scientist)) Message("$Scientist1$", scientist);
  scientist -> Sound("Scream");
}

func Script30()
{
  if (GetAlive(scientist)) Message("$Scientist2$", scientist);
  scientist -> Jump();
}

func Script35()
{
  scientist -> Jump();
}

func Script40()
{
  if (GetAlive(scientist)) Message("$Scientist3$", scientist);
}

func InitializePlayer(player)
{ 
  SetFoW(true, player);
  var sft = GetCrew(player);

  sft -> SetPosition(2350 + Random(100), 680);
  // sft -> SetPlrViewRange(250);
  sft -> CreateContents(PM5B);
  sft -> CreateContents(PM5B);
  sft -> CreateContents(NH5B);

  // das Sicherheitssystem mag die Spieler nicht
  var isi = GetPlayerByName("I.S.I");
  SetHostility(isi, player, true, true);
}

func GetPlayerByName(name)
{
  for(var i = 0; i < GetPlayerCount(); i++)
  {
    if (GetPlayerName(GetPlayerByIndex(i)) == name) return i;
  }
  return(-1);
}