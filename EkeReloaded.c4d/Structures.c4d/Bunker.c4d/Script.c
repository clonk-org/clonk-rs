/* Bunker */

#strict

#include B35B
#include DOOR

local turret;
local delay;

func Construction()
{
  SetCon(3);
  return(_inherited());
}

func SoundOpenDoor()
{
  Sound("BK_DoorOpen");
}

func SoundCloseDoor()
{
  Sound("BK_DoorClose");
}

func ContainedLeft(clonk)
{
  var baseOwner = GetBase();
  if (baseOwner < 0)                          return(1);
  if (Hostile(baseOwner, GetOwner(clonk), 1)) return(1);
  if (delay)                                  return(1);

  var i = 0; var object;
  while (object = Contents(i))
  {
    // kein anderer Clonk darf in diesem Menü sein
    if (GetMenu(object) == GetID()) return(1);
    i++;
  }
  if (turret)
  {
    if (GetAction(turret) ne "Rotate") return(1);
    if (LocalN("deinstall", turret))   return(1);

    var turretId = GetID(turret);
    var caption = Format("$Deinstall$", GetName(0, turretId));

    CreateMenu(GetID(), clonk, 0, 2);
    AddMenuItem(caption, "DeinstallTurret", turretId, clonk, 0, clonk, GetDesc(0, turretId));
  }
  else
  {  
    var caption1 = Format("$Install$", GetName(0, FK5B));
    var caption2 = Format("$Install$", GetName(0, PC5B));
    var caption3 = Format("$Install$", GetName(0, TC5B));

    CreateMenu(GetID(), clonk, 0, 2);

    AddMenuItem(caption1, "InstallTurret", FK5B, clonk, 0, clonk, GetDesc(0, FK5B));
    AddMenuItem(caption2, "InstallTurret", PC5B, clonk, 0, clonk, GetDesc(0, PC5B));
    AddMenuItem(caption3, "InstallTurret", TC5B, clonk, 0, clonk, GetDesc(0, TC5B));
  }
  return(1);
}

func ContextTurret(clonk)
{
  [$Context$|Image=BK5B]
  clonk -> ContainedCall("ContainedLeft", this());
}

func InstallTurret(turretId, clonk)
{
  var value = GetValue(0, turretId);
  var baseOwner = GetBase();

  if (GetWealth(baseOwner) < value)
  {
    PlayerMessage(baseOwner, "$Money$", clonk);
    Sound("CommandFailure1");
    return;  
  }
  DoWealth(baseOwner, -value);
  Sound("UnCash");

  // verzögert, damit der Cash-Sound gut hörbar ist
  ScheduleCall(0, "DelayedInstall", 10, 0, turretId);
  delay = true;
}

func DeinstallTurret(turretId, clonk)
{
  var value = GetValue(0, turretId);
  var baseOwner = GetBase();

  DoWealth(baseOwner, value);
  Sound("Cash");

  // verzögert, damit der Cash-Sound gut hörbar ist
  ScheduleCall(0, "DelayedDeinstall", 10);
  delay = true;
}

func DelayedInstall(turretId)
{
  turret = CreateObject(turretId, 0, -13, NO_OWNER);
  LocalN("bunker", turret) = this;
  delay = false;
}

func DelayedDeinstall()
{
  LocalN("deinstall", turret) = true;
  delay = false;
}

func Detonate()
{
  var object;
  while (object = Contents())
  {
    Exit(object);
  }
  if (turret)
  {
    Explode(35, turret);
    Schedule("Explode(75)", 10);
  }
  else
  {
    Explode(75);
  }
  return(1);
}

func Damage()
{  
  if (GetDamage() < (GetMass() / 20)) return(1);

  var basement = FindObject(B35B, 0, 14);
  if (!basement)
  {
    // Pech gehabt, sofortige Sprengung
    Schedule("Detonate()", 1);
  }
  else
  {
    // Alarm und Countdown, dann Sprengung
    basement -> SetAction("Alarm", this());
  }
  return(1);
}