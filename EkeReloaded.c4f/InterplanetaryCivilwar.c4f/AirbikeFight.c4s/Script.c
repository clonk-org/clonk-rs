/* AirbikeFight */

#strict

func Initialize()
{
  SetSkyParallax(0, 17, 19, 0, 0, 0, 0);
  SetGamma(RGB(15, 15, 15), RGB(118, 118, 118), RGB(215, 215, 215));

  // Nebel erzeugen
  for(var i = 0; i < 180; ++i)
  {
    var x = Random(LandscapeWidth());
    var y = Random(LandscapeHeight());
    var s = RandomX(900, 1700);
    
    CreateParticle("Fog", x, y, 0, 0, s);
  }
  ScriptGo(1);
}

func Script1()
{
  SetMaxPlayer();
  return(1);
}

private func InitializeClonk(clonk)
{ 
  // Clonk ausrüsten
  //***************** 
  var jetpack = CreateContents(JP5B, clonk);
  LocalN("ammo", jetpack) = 100;

  CreateContents(GP5B, clonk);
  CreateContents(CA5B, clonk);

  var xPos = Random(LandscapeWidth());
  var yPos = Random(LandscapeHeight());
  var airbike = CreateObject(AB5B, xPos, yPos, GetOwner(clonk));
  
  // Position anpassen 
  while (Stuck(airbike))
  {
    xPos = Random(LandscapeWidth());
    yPos = Random(LandscapeHeight());
    SetPosition(xPos, yPos, airbike);
  }
  LocalN("ammo", airbike) = 100;
  LocalN("qRockets", airbike) = 100;
  LocalN("qBombs", airbike) = 100;

  var blaster = CreateContents(OB5B, airbike);
  SetPicture(35, 7, 35, 35, blaster);
  LocalN("spikes", blaster) = 1;

  airbike -> ControlRequest(clonk);

  SetDir(Random(2), clonk);
  SetDir(GetDir(clonk), airbike);
  SetComDir(COMD_Up(), airbike);
  return(1);
}