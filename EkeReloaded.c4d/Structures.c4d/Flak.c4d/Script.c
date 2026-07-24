/* Flak */

#strict

local reloadTime;
local deinstall;
local bunker;

func Initialize()
{
  SetAction("PrepareInstall");
  Sound("BK_TurretStart");
}

func Deinstall()
{
  SetAction("PrepareDeinstall");
  Sound("BK_TurretStart");
}

func Installed()
{
  SetAction("Rotate");
  SetPhase(20);
  Sound("BK_TurretStop");
}

func Deinstalled()
{
  RemoveObject();
  Sound("BK_TurretStop");
}

func TargetScan()
{
  if (GetAction() ne "Rotate") return;
  if (RotateForDeinstall())    return;
  if (InLiquid())              return;
  
  var target = GetTarget();
  if (target) LaunchMissile(GetAngle(target), target);  
}

func RotateForDeinstall()
{
  if (!deinstall) return;
  if (!Aim(210)) Deinstall();
 
  return(1);
}

func GetTarget()
{
  var a1 = Find_InRect(-500, -GetY(), 1000, GetY());
  var a2 = Find_NoContainer();
  var a3 = Find_PathFree();
  var a4 = Sort_Distance();

  for (var target in FindObjects(a1, a2, a3, a4))
  {
    var b1 = Hostile(GetController(bunker), GetController(target), 1);

    var b2 = GetOCF(target) & OCF_Grab();
    var b3 = GetOCF(target) & OCF_Alive();
    var b4 = GetOCF(target) & OCF_Container();
    
    if (b1 && (b2 || b3 || b4)) return(target);
  }
  return;
}

func GetAngle(target)
{
  var x1 = GetX(target);
  var y1 = GetY(target);
  var x2 = GetX();
  var y2 = GetY();

  var angle = Angle(x1, y1, x2, y2);

  angle = Max(angle, 90);
  angle = Min(angle, 270);

  return(angle);
}

func Aim(angle)
{
  var targetPhase = (angle - 90) / 6;
  var actualPhase = GetPhase();

  var delta = targetPhase - actualPhase;
  var change = delta / Abs(delta);

  SetPhase(actualPhase + change);

  return(delta);
}

func LaunchMissile(angle, target)
{
  reloadTime++;

  if ((reloadTime > 100) && (!Aim(angle)))
  {
    var dif = 15 - GetPhase();
    var dir = dif / Abs(dif);

    var x = Cos(angle, 17) * (2 * (angle > 180) -1) - Sin(angle, 5);
    var y = Cos(angle, 5);
    var r = 180 + angle + Max(0, Abs(dif) - 10) * dir;

    var missile = CreateObject(MS5B, x, y, NO_OWNER);
    SetR(r, missile);
    missile -> Launch(this, target);
    SetController(GetController(bunker), missile);

    reloadTime = 0;
  }
}