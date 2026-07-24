/* OilWars */

#strict

func GetSftSuit() { return("Desert"); }

func Initialize()
{
  var tower1 = CreateObject(TWR2, 1005, 306, -1);
  var tower2 = CreateObject(TWR2, 1227, 306, -1);

  var gate1 = FindObject(0, 0, 0, 0, 0, 0, "GateRight", tower1);
  var gate2 = FindObject(0, 0, 0, 0, 0, 0, "GateLeft", tower2);
  
  gate1 -> Activate();
  gate2 -> Activate();

  ScriptGo(1);
}

func Script1()
{
  SetMaxPlayer();
}