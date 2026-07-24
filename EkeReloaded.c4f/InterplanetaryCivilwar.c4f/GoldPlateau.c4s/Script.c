/* GoldPlateau */

#strict

func Initialize()
{
  ScriptGo(1);
  return(1);
}

func Script1()
{
  SetMaxPlayer();
  return(1);
}

func Script1000()
{   
  ExpandHomebaseStuff(UZ5B);
  ExpandHomebaseStuff(SG5B);
  ExpandHomebaseStuff(CA5B);
  return(1);
}

func Script1500()
{   
  ExpandHomebaseStuff(BB5B);
  ExpandHomebaseStuff(FT5B);
  ExpandHomebaseStuff(GS5B);
  return(1);
}

func Script2000()
{   
  ExpandHomebaseStuff(HP5B);
  ExpandHomebaseStuff(AR5B);
  ExpandHomebaseStuff(GB5B);
  return(1);
}

func Script2500()
{   
  ExpandHomebaseStuff(CANN);
  ExpandHomebaseStuff(BLMP);
  return(1);
}

func Script3500()
{   
  ExpandHomebaseStuff(JP5B);
  ExpandHomebaseStuff(GP5B);
  return(1);
}

func Script4500()
{   
  ExpandHomebaseStuff(RL5B);
  ExpandHomebaseStuff(RB5B);
  return(1);
}

func Script5000()
{   
  ExpandHomebaseStuff(AB5B);
  return(1);
}

func ExpandHomebaseStuff(objectId)
{
  for (var i = 0; i < GetPlayerCount(); i++)
  {
    DoHomebaseMaterial(GetPlayerByIndex(i), objectId, 3);
    DoHomebaseProduction(GetPlayerByIndex(i), objectId, 3);
  }
}