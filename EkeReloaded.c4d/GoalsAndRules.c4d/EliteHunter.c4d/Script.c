/* Elite Hunter */

#strict
#include GOAL

local target;
local finished;

func Initialize()
{
  var goal;
  if (goal = FindObject(GetID()))
  {
    LocalN("target", goal) += 100;
    return(RemoveObject());
  }
  target = 100;
  return(inherited());
}

func InitializePlayer(player)
{
  // für genügend Stippel sorgen
  var count = GetPlayerCount(C4PT_User) * target;
  while (ObjectCount(ST5B) < count) PlaceAnimal(ST5B);

  // Scoreboard erstellen
  var id = GetPlayerID(player);

  SetScoreboardData(SBRD_Caption, SBRD_Caption, "Stippelkills", SBRD_Caption);
  SetScoreboardData(SBRD_Caption, 1, "{{ST5B}}", 1);
  SetScoreboardData(id, SBRD_Caption, GetTaggedPlayerName(player), id);
  SetScoreboardData(id, 1, Format("%d/%d", 0, target), 0);
  SortScoreboard(1, true);

  DoScoreboardShow(1, player + 1);
}
    
func IsFulfilled() { return(finished); }

func Activate(player)
{
  var rest = target - Global(player);
  MessageWindow(Format("$RestInfo$", rest), player);
}

func ReportHomicide(killer, victim, vId, unborn)
{
  var killerID = GetPlayerID(killer);
  
  if (GetPlayerName(victim) ne "Stippel") return; // kein Stippel
  if (GetPlayerName(killer) eq "Stippel") return; // Selbstmord
  if (!killerID)                          return; // höhere Gewalt
  if (unborn)                             return; // Miffel am Baum zählen nicht

  Global(killer)++;
  var kills = Global(killer);
  if (kills == target) GameOver(killer);

  SetScoreboardData(killerID, 1, Format("%d/%d", kills, target), kills);
  SortScoreboard(1, true);
}

func GameOver(winner)
{
  for(var i = 0; i < GetPlayerCount(C4PT_User); i++)
  {
    var index = GetPlayerByIndex(i, C4PT_User);
    if (index != winner) EliminatePlayer(index);
  }
  var message = Format("<c ffff00>$GameOver$</c>", GetPlayerName(winner));
  CustomMessage(message, 0, NO_OWNER, 0, -50);
  finished = true;
}