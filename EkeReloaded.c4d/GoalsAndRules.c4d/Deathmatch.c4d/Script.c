/* Deathmatch */

#strict
#include MELE

local relaunchs;
local kills;
local selfkills;
local flipflop;

protected func Initialize()
{
  relaunchs = CreateArray();
  kills = CreateArray();
  selfkills = CreateArray();
  flipflop = CreateArray();

  return(inherited());
}

func InitializePlayer(player, x, y, base, team)
{
  var playerID = GetPlayerID(player);

  SetScoreboardData(SBRD_Caption, SBRD_Caption, "Score", SBRD_Caption);

  SetScoreboardData(SBRD_Caption, 1, "{{SF5B}}", 1);
  SetScoreboardData(SBRD_Caption, 2, "{{UZ5B}}", 2);
  SetScoreboardData(SBRD_Caption, 3, "{{HG5B}}", 3);
  SetScoreboardData(playerID, SBRD_Caption, GetTaggedPlayerName(player), playerID);
  SetScoreboardData(playerID, 1, "10", 10);
  SetScoreboardData(playerID, 2, "0", 0);
  SetScoreboardData(playerID, 3, "0", 0);
  SortScoreboard(1, true);

  DoScoreboardShow(1, player + 1);

  GameCall("InitializeClonk", GetCrew(player));
  return(inherited(player, x, y, base, team));
}

func Activate(player)
{
  var playerID = GetPlayerID(player);

  var r = 10 - relaunchs[playerID];
  var k = kills[playerID];
  var s = selfkills[playerID];

  MessageWindow(Format("$Info$", r, k, s), player);
}

func RelaunchPlayer(player)
{
  var playerID = GetPlayerID(player);

  relaunchs[playerID]++;
  flipflop[playerID] = false;

  var rest = 10 - relaunchs[playerID];
  if (rest < 0) return;

  SetScoreboardData(playerID, 1, Format("%d", rest), rest);
  SetScoreboardData(playerID, 2, Format("%d", kills[playerID]), kills[playerID]);
  SortScoreboard(1, true);

  var clonk = CreateObject(SF5B, 0, 0, player);
  MakeCrewMember(clonk, player);
  clonk -> DoEnergy(100);
  SelectCrew(player, clonk, 1);

  GameCall("InitializeClonk", clonk);
  return(1);
}

func ReportHomicide(killer, victim)
{
  var killerID = GetPlayerID(killer);
  if (!killerID) return;
  
  if (killer == victim)
  {
    selfkills[killerID]++;
    SetScoreboardData(killerID, 3, Format("%d", selfkills[killerID]), selfkills[killerID]);
  }
  else
  {
    kills[killerID]++;

    flipflop[killerID] = !flipflop[killerID];
    if (!flipflop[killerID])
    {
      relaunchs[killerID]--;
      var rest = 10 - relaunchs[killerID];

      SetScoreboardData(killerID, 1, Format("%d", rest), rest);
      SetScoreboardData(killerID, 2, Format("%d", kills[killerID]), kills[killerID]);
      SortScoreboard(1, true);
    }
    else
    {
      // rote Farbe signalisiert, das beim nächsten Kill ein Relaunch gewonnen wird
      SetScoreboardData(killerID, 2, Format("<c ff0000>%d</c>", kills[killerID]), kills[killerID]);
    }
  }
}

protected func RemovePlayer(player)
{
  var playerID = GetPlayerID(player);

  SetScoreboardData(playerID, 1, 0, 0);
  SetScoreboardData(playerID, 2, 0, 0);
  SetScoreboardData(playerID, 3, 0, 0);
}