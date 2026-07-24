/* Jetpack */

#strict

// restlicher Treibstoff in Prozent, Bezeichnung "ammo" wird vom SFT benötigt
local ammo;

func ControlThrow(object clonk)
{ 
  var a1 = GetAction(clonk) eq "JetpackFly";
  var a2 = GetAction(clonk) eq "JetpackWalk";
  var a3 = GetAction(clonk) eq "JetpackJump";
  var a4 = GetAction(clonk) eq "Tumble";
   
  if (!a1 && !a2 && !a3 && !a4) return(1);

  if (!ammo)
  {
    Sound("FT_Empty");
    return(1);
  }
  if (a1)
  {
    StopThrust(clonk);
    return(1);
  }
  if (!a2) 
  {
    clonk -> SetAction("JetpackFly");

    var x = -16 * GetDir(clonk) + 8 ;
    var y = 10;

    Smoke(x, y + Random(9), 9);
    Smoke(x, y + Random(9), 7);
    Smoke(x, y + Random(9), 5);
    Smoke(x, y + Random(9), 3);

    Sound("JP_Launch");
  }
  return(1);
}

func ControlLeft(object clonk)
{ 
  if (GetAction(clonk) ne "JetpackFly") return(0);
  SetDir(DIR_Left(), clonk);
  SetComDir(COMD_Left(), clonk);
  return(1);
}

func ControlRight(object clonk)
{ 
  if (GetAction(clonk) ne "JetpackFly") return(0);
  SetDir(DIR_Right(), clonk);
  SetComDir(COMD_Right(), clonk);
  return(1);
}

func ControlUp(object clonk)
{ 
  if (GetAction(clonk) ne "JetpackFly") return(0);
  SetComDir(COMD_Up(), clonk);
  return(1);
}

func ControlDown(object clonk)
{ 
  if (GetAction(clonk) ne "JetpackFly") return(0);
  SetComDir(COMD_Down(), clonk);
  return(1);
}

func StopThrust(object clonk)
{
  SetComDir(COMD_None(), clonk);
  clonk -> SetAction("JetpackJump");
  return(1);
}

func Flying(object clonk)
{
  if (!ammo)
  {
    StopThrust(clonk);
    Sound("FT_Empty");
    return(1);
  }
  var x = -16 * GetDir(clonk) + 8 ;
  var y = 10;
  
  // Rauch, wenn der Treibstoff aus geht
  if (ammo < 10) Smoke(x, y, 10 - ammo);

  // Triebwerk-Strahl-Effekt
  x += GetDir(clonk);
  CreateParticle("EkeAfterburner1", x, 3, 0, 0, 90, RGBa(255, RandomX(50, 100), 0, 150), clonk);

  ammo--;
  clonk -> SetAmmoBar(ammo);
  return(1);
}

func Activate(object clonk)
{
  if (ammo == 100) return(0);  

  var gas = FindContents(GS5B, clonk);
  if (!gas) return(0);

  RemoveObject(gas);
  
  if (GetAction(clonk) eq "JetpackFly")
  {
    StopThrust(clonk);
  }
  ammo = 100;
  clonk -> SetAmmoBar(ammo);

  Sound("FT_Reload");
  return(1);
}