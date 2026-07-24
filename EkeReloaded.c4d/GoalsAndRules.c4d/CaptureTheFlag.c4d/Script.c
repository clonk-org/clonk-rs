/* Capture The Flag */

#strict
#include MELE

local bunker1;
local bunker2;
local bunker3;
local bunker4;

local casualties;

// Team ID 1 -> New Republic
// Team ID 2 -> Union

protected func Initialize()
{
  bunker1 = GameCall("GetBunker1");
  bunker2 = GameCall("GetBunker2");
  bunker3 = GameCall("GetBunker3");
  bunker4 = GameCall("GetBunker4");

  SetName(GameCall("GetBunker1Name"), bunker1);
  SetName(GameCall("GetBunker2Name"), bunker2);
  SetName(GameCall("GetBunker3Name"), bunker3);
  SetName(GameCall("GetBunker4Name"), bunker4);

  for (var flagPole in FindObjects(Find_ID(FP5B)))
  {
    var color = RGB(255, 255, 255);

    if (ObjectDistance(flagPole, bunker1) < 200) color = GetTeamColor(1);
    if (ObjectDistance(flagPole, bunker4) < 200) color = GetTeamColor(2);
    
    flagPole -> CreateFlag(color);
  }
  InitializeScoreboard();
  return(inherited());
}

func InitializeScoreboard()
{
  casualties = [0, 0];

  SetScoreboardData(SBRD_Caption, SBRD_Caption, "Score");  

  SetScoreboardData(1, SBRD_Caption, Format("<c ff0000>%s</c>", GetTeamName(1)));
  SetScoreboardData(2, SBRD_Caption, Format("<c 0000ff>%s</c>", GetTeamName(2)));

  SetScoreboardData(SBRD_Caption, 1, "{{BK5B}}");
  SetScoreboardData(SBRD_Caption, 2, "{{SY5B}}");

  SetScoreboardData(1, 1, "1");
  SetScoreboardData(2, 1, "1");

  SetScoreboardData(1, 2, "0");
  SetScoreboardData(2, 2, "0");
}

func InitializePlayer(player, x, y, base, team)
{
  SetOwner(GetPlayerByTeam(1), bunker1);
  SetOwner(GetPlayerByTeam(2), bunker4);

  DoScoreboardShow(1, player + 1);

  var clonk = GetCrew(player);
  InitializeClonk(clonk);
  return(inherited(player, x, y, base, team));
}

func InitializeClonk(clonk, homeCount)
{ 
  var sanctuary = CreateObject(SY5B, 0, 0, NO_OWNER);
  SetPosition(LandscapeWidth() / 2, 0, sanctuary);
  Enter(sanctuary, clonk);

  CreateMenu(SY5B, clonk, 0, 0, "R.I.P", 0, C4MN_Style_Info);

  LocalN("homeCount", sanctuary) = homeCount;
  sanctuary -> SetAction("Countdown", clonk);
}

func ChooseGear(clonk)
{
  CloseMenu(clonk); // Relaunch-Countdown-Menü schliessen
  CreateMenu(SF5B, clonk, 0, 0, "SFT Gear", 0, C4MN_Style_Context);
  
  AddMenuItem("Marine", "GearMarine", AR5B, clonk, 0, clonk);
  AddMenuItem("Grillmaster", "GearGrillmaster", FT5B, clonk, 0, clonk);
  AddMenuItem("Missileman", "GearMissileman", RL5B, clonk, 0, clonk);
  AddMenuItem("Blasterboss", "GearBlasterboss", SG5B, clonk, 0, clonk);
  AddMenuItem("Airlord", "GearAirlord", JP5B, clonk, 0, clonk);
  AddMenuItem("Arrowagent", "GearArrowagent", HP5B, clonk, 0, clonk);
}

func RelaunchPlayer(player)
{
  var team = GetPlayerTeam(player);
  casualties[team - 1]++;
  SetScoreboardData(team, 2, Format("%d", casualties[team - 1]));

  var b1 = GetPlayerTeam(GetOwner(bunker1)) == GetPlayerTeam(player);
  var b2 = GetPlayerTeam(GetOwner(bunker2)) == GetPlayerTeam(player);
  var b3 = GetPlayerTeam(GetOwner(bunker3)) == GetPlayerTeam(player);
  var b4 = GetPlayerTeam(GetOwner(bunker4)) == GetPlayerTeam(player);

  // alle Bunker gehören dem Feind?
  if (!b1 && !b2 && !b3 && !b4)
  {
    // kein verbündeter SFT in freier Wildbahn?
    if (!FindObject2(Find_OCF(OCF_CrewMember), Find_Allied(player), Find_NoContainer()))
    {
      // das ganze Team terminieren!
      for (var i = GetPlayerCount(); i--;)
      {
        var p = GetPlayerByIndex(i);    
        if (GetPlayerTeam(p) == GetPlayerTeam(player))
        {
          var content;
          while (content = Contents(0, GetCursor(p)))
          {
            // Clonk vorm Killen entleeren
            RemoveObject(content);
          }        
          EliminatePlayer(p);
        }
      }
      return;
    }
  }
  var clonk = CreateObject(SF5B, 0, 0, player);
  MakeCrewMember(clonk, player);
  clonk -> DoEnergy(100);
  SelectCrew(player, clonk, 1);
    
  var homeCount = 25 - ((b1 + b2 + b3 + b4) * 5);
  InitializeClonk(clonk, homeCount);
}

func CreateAirbike(team, bunker, xOffset, yOffset, direction)
{
  var player = GetPlayerByTeam(team);

  var b1 = GetPlayerTeam(GetOwner(bunker)) == team;
  var b2 = !FindObject2(Find_ID(AB5B), Find_Allied(player));
  var b3 = player >= 0;

  if (b1 && b2 && b3)
  {
    var airbike = CreateObject(AB5B, 0, 0, player);
    SetPosition(GetX(bunker) + xOffset, GetY(bunker) + yOffset, airbike);
    SetDir(direction, airbike);

    LocalN("ammo", airbike) = 100;
    LocalN("qRockets", airbike) = 100;
    LocalN("qBombs", airbike) = 100;
  }
}

func GearMarine(icon, clonk)
{
  var assaultRifle = CreateContents(AR5B, clonk); 
  LocalN("ammo", assaultRifle) = 100;
  LocalN("qGrenades", assaultRifle) = 100;  

  CreateContents(HG5B, clonk);
  CreateContents(HG5B, clonk);
  CreateContents(HG5B, clonk);
  CreateContents(NH5B, clonk);
  
  ChooseBunker(clonk);
}

func GearGrillmaster(icon, clonk)
{
  var flamethrower = CreateContents(FT5B, clonk);
  LocalN("ammo", flamethrower) = 100;  

  CreateContents(MO5B, clonk);
  CreateContents(MO5B, clonk);
  CreateContents(MO5B, clonk);
  CreateContents(NH5B, clonk);
  
  ChooseBunker(clonk);
}

func GearMissileman(icon, clonk)
{
  var rocketLauncher = CreateContents(RL5B, clonk);
  rocketLauncher -> CreateContents(MS5B);
  LocalN("ammo", rocketLauncher) = 100;  
  
  CreateContents(MS5B, clonk);
  CreateContents(MS5B, clonk);
  CreateContents(MS5B, clonk);
  CreateContents(NH5B, clonk);

  ChooseBunker(clonk);
}

func GearBlasterboss(icon, clonk)
{
  var shotgun = CreateContents(SG5B, clonk); 
  LocalN("ammo", shotgun) = 100;  

  CreateContents(GP5B, clonk);
  CreateContents(BB5B, clonk);
  CreateContents(NH5B, clonk);
 
  ChooseBunker(clonk);
}

func GearAirlord(icon, clonk)
{
  var jetpack = CreateContents(JP5B, clonk); 
  LocalN("ammo", jetpack) = 100;

  var uzi = CreateContents(UZ5B, clonk); 
  LocalN("ammo", uzi) = 100;

  CreateContents(GS5B, clonk);
  CreateContents(NH5B, clonk);
  
  ChooseBunker(clonk);
}

func GearArrowagent(icon, clonk)
{
  var harpoon = CreateContents(HP5B, clonk); 
  LocalN("qGrenades", harpoon) = 100;

  CreateContents(GB5B, clonk);
  CreateContents(PM5B, clonk);
  CreateContents(PM5B, clonk);
  CreateContents(NH5B, clonk);
 
  ChooseBunker(clonk);
}

func ChooseBunker(clonk)
{
  var b1 = GetPlayerTeam(GetOwner(bunker1)) == GetPlayerTeam(GetOwner(clonk));
  var b2 = GetPlayerTeam(GetOwner(bunker2)) == GetPlayerTeam(GetOwner(clonk));
  var b3 = GetPlayerTeam(GetOwner(bunker3)) == GetPlayerTeam(GetOwner(clonk));
  var b4 = GetPlayerTeam(GetOwner(bunker4)) == GetPlayerTeam(GetOwner(clonk));

  CreateMenu(BK5B, clonk, 0, 0, "Entry Point", 0, C4MN_Style_Context, true);

  AddMenuItem(GetName(bunker1), "EnterBunker1", SY5B, clonk, 0, clonk, 0, 2, !b1 + 1);
  AddMenuItem(GetName(bunker2), "EnterBunker2", SY5B, clonk, 0, clonk, 0, 2, !b2 + 1);
  AddMenuItem(GetName(bunker3), "EnterBunker3", SY5B, clonk, 0, clonk, 0, 2, !b3 + 1);
  AddMenuItem(GetName(bunker4), "EnterBunker4", SY5B, clonk, 0, clonk, 0, 2, !b4 + 1);

  SelectMenuItem(0, clonk);
}

func UpdateBunkerMenu()
{
  for (var i = GetPlayerCount(); i--;)
  {
    var clonk = GetCursor(GetPlayerByIndex(i));
    var team = GetPlayerTeam(GetOwner(clonk));

    var b1 = GetPlayerTeam(GetOwner(bunker1)) == GetPlayerTeam(GetOwner(clonk));
    var b2 = GetPlayerTeam(GetOwner(bunker2)) == GetPlayerTeam(GetOwner(clonk));
    var b3 = GetPlayerTeam(GetOwner(bunker3)) == GetPlayerTeam(GetOwner(clonk));
    var b4 = GetPlayerTeam(GetOwner(bunker4)) == GetPlayerTeam(GetOwner(clonk));
    var b5 = FindObject2(Find_OCF(OCF_CrewMember), Find_Allied(GetOwner(clonk)), Find_NoContainer());
    
    // Scoreboard aktualisieren
    SetScoreboardData(GetPlayerTeam(GetOwner(clonk)), 1, Format("%d", b1 + b2 + b3 + b4));

    // alle Bunker gehören dem Feind und kein verbündeter SFT in freier Wildbahn?
    if (!b1 && !b2 && !b3 && !b4 && !b5)
    {
      var content;
      while (content = Contents(0, clonk))
      {
        // Clonk vorm Killen entleeren
        RemoveObject(content);
      }
      EliminatePlayer(GetOwner(clonk));
    }
    else
    {
      if (GetMenu(clonk) == BK5B)  
      {
        var index = GetMenuSelection(clonk);
        ClearMenuItems(clonk);
    
        AddMenuItem(GetName(bunker1), "EnterBunker1", SY5B, clonk, 0, clonk, 0, 2, !b1 + 1);
        AddMenuItem(GetName(bunker2), "EnterBunker2", SY5B, clonk, 0, clonk, 0, 2, !b2 + 1);
        AddMenuItem(GetName(bunker3), "EnterBunker3", SY5B, clonk, 0, clonk, 0, 2, !b3 + 1);
        AddMenuItem(GetName(bunker4), "EnterBunker4", SY5B, clonk, 0, clonk, 0, 2, !b4 + 1);

        SelectMenuItem(index, clonk);
      }
    }
  }  
}

func EnterBunker1(icon, clonk) { EnterBunker(clonk, bunker1); }
func EnterBunker2(icon, clonk) { EnterBunker(clonk, bunker2); }
func EnterBunker3(icon, clonk) { EnterBunker(clonk, bunker3); }
func EnterBunker4(icon, clonk) { EnterBunker(clonk, bunker4); }

func EnterBunker(clonk, bunker)
{
  if (GetPlayerTeam(GetOwner(bunker)) == GetPlayerTeam(GetOwner(clonk)))
  {  
    CloseMenu(clonk);

    var sanctuary = Contained(clonk);
    Enter(bunker, clonk);

    RemoveObject(sanctuary);
  }
  else Sound("Error", 0, 0, 0, GetOwner(clonk) + 1);
}

func OnMenuSelection(index, clonk)
{
  if (GetMenu(clonk) == BK5B)  
  {
    if (index == 0) SetPlrView(GetOwner(clonk), bunker1);
    if (index == 1) SetPlrView(GetOwner(clonk), bunker2);
    if (index == 2) SetPlrView(GetOwner(clonk), bunker3);
    if (index == 3) SetPlrView(GetOwner(clonk), bunker4);
  }
}

func MenuQueryCancel()
{ 
  return(true);
}

func GetPlayerByTeam(teamId)
{
  for(var i = 0; i < GetPlayerCount(); i++)
  {
    if (GetPlayerTeam(GetPlayerByIndex(i)) == teamId) return i;
  }
  return(-1);
}

func Activate(player)
{
  var b1 = GetPlayerTeam(GetOwner(bunker1)) == GetPlayerTeam(player);
  var b2 = GetPlayerTeam(GetOwner(bunker2)) == GetPlayerTeam(player);
  var b3 = GetPlayerTeam(GetOwner(bunker3)) == GetPlayerTeam(player);
  var b4 = GetPlayerTeam(GetOwner(bunker4)) == GetPlayerTeam(player);

  var rest = 4 - (b1 + b2 + b3 + b4);
  MessageWindow(Format("$RestInfo$", rest), player);
}