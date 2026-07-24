/* SpeedHunting */

#strict

static bunker;

func Initialize()
{
  bunker = CreateObject(BK5B, LandscapeWidth() / 2, 66);
  bunker -> DelayedInstall(PC5B);

  ScriptGo(1);
}

func InitializePlayer(player)
{
  Enter(bunker, GetCrew(player));
}

func Script1()
{
  SetMaxPlayer();
}