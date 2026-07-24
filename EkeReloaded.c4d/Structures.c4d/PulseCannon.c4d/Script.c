/* Pulskanone */

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
  SetPhase(30);
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
  if (target) LaunchPulse(GetAngle(target), target);  
}

func RotateForDeinstall()
{
  if (!deinstall) return;
  if (!Aim(90)) Deinstall();
 
  return(1);
}

func GetTarget()
{
  var a1 = Find_InRect(-250, 0, 500, 100);
  var a2 = Find_Hostile(GetController(bunker));
  var a3 = Find_OCF(OCF_Alive);
  var a4 = Find_NoContainer();
  var a5 = Find_PathFree();
  var a6 = Sort_Distance();

  var target = FindObject2(a1, a2, a3, a4, a5, a6);
  return(target);
}

func GetAngle(target)
{
  var x1 = GetX(target);
  var y1 = GetY(target);
  var x2 = GetX();
  var y2 = GetY();

  var angle = Angle(x2, y2, x1, y1);

  angle = Max(angle, 90);
  angle = Min(angle, 270);

  return(angle);
}

func Aim(angle)
{
  var targetPhase = (270 - angle) / 6;
  var actualPhase = GetPhase();

  var delta = targetPhase - actualPhase;
  var change = delta / Abs(delta);

  SetPhase(actualPhase + change);

  return(delta);
}

func LaunchPulse(angle, target)
{
  reloadTime++;

  if ((reloadTime > 25) && (!Aim(angle)))
  {
    var x = Sin(angle, 20) - 1;
    var y = Cos(angle, -15) - 2;

    var xDir = +Sin(angle, 10);
    var yDir = -Cos(angle, 10);

    var pulse = CreateContents(PU5B);
    Exit(pulse, x, y, angle, xDir, yDir);
    SetController(GetController(bunker), pulse);

    reloadTime = 0;
  }
}