/* Uzi */

#strict

// Restmunition in Prozent
local ammo;

func ControlThrow(object clonk)
{  
  if (!WildcardMatch(GetAction(clonk), "Uzi*")) return(1);

  // zu schnelles Feuern unterbinden
  if (GetAction() eq "Stop") return(1);

  if (!ammo)
  {
    Sound("PT_Empty");
    return(1);
  }  
  if (GetAction() eq "Shoot")
  {
    SetAction("Stop");
  }
  else
  {
    SetAction("Shoot");
  }
  return(1);
}

func Shooting()
{
  var clonk = Contained();
  var dir = GetDir(clonk);  

  var b1 = !WildcardMatch(GetAction(clonk), "Uzi*");
  var b2 = Contents(0, clonk) != this();
  var b3 = Contained(clonk);

  if (b1 || b2 || b3)
  {
    SetAction("Stop");
    return(1);
  }
  if (!ammo)
  {
    SetAction("Stop");
    Sound("PT_Empty");
    return(1);
  } 
  CreateBullet(dir, clonk);
  CreateMuzzleFlash(dir, clonk);
  CreateShell(dir);
 
  ammo--;
  clonk->SetAmmoBar(ammo);
  return(1);
}

func Activate(object clonk)
{
  if (ammo == 100) return(0);  

  var cartridges = FindContents(CA5B, clonk);
  if (!cartridges) return(0);
  
  RemoveObject(cartridges);
  SetAction("Stop");

  ammo = 100;
  clonk -> SetAmmoBar(ammo);

  Sound("UZ_Reload");
  return(1);
}

func CreateBullet(dir, clonk)
{
  var x = 8 * dir - 4;
  var y = 0;

  var bullet = CreateObject(BU5B, x, y, NO_OWNER);
  bullet -> Launch(this(), dir, clonk);
  SetController(GetController(clonk), bullet);
  return(1);
}

func CreateMuzzleFlash(dir, clonk)
{
  var x = 30 * dir - 15;
  var y = -3 + dir;
  var a = 180 * dir - 90;

  CreateParticle("EkeMuzzleFlash1", x, y, Sin(a, 1000), -Cos(a, 1000), 85, 0, clonk);
  return(1);
}

func CreateShell(dir)
{
  var x = 10 * dir - 5 - dir;
  var y = -1;
  
  var shell = CreateObject(SH5B, x, y);
  shell -> Launch(dir);
  return(1);
}