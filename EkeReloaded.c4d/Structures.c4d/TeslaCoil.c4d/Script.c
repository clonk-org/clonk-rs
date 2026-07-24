/* Teslaspule */

#strict

local targets;
local actTime;
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

  actTime++;

  AimToPole();

  if (actTime == 25)
  {
    targets = CreateArray();
    targets = GetTargets();
    if (GetLength(targets)) SetPhase(0);
    else actTime = 24;
  }
  if (actTime == 26) Sound("TC_Strike");
  if (actTime > 026) AimToTarget();
  if (actTime > 100) actTime = 0;
}

func AimToPole()
{
  var phase = GetPhase();

  if (phase < 5)                return;
  if (phase > 29 && Random(10)) return;

  var x1 = GetX() - 9 - (phase / 2);
  var x2 = GetX() - 4 - (phase / 5);
  var x3 = GetX() + 4 + (phase / 5);
  var x4 = GetX() + 9 + (phase / 2);

  var y = GetY() - 17 + (phase / 2);

  var xArray = [x1, x2, x3, x4];

  CreateLightning(xArray[Random(4)], y, false);
  
  // würde mit Sound nerven glaub ich...
  // if (phase > 29) Sound("arc*");
}

func AimToTarget()
{
  var index = Max(actTime, 30) - 30;
  if ((index < GetLength(targets)) && (actTime < 40))
  {
    var target = targets[index];
    var dir = GetX(target) > GetX();

    var repeat = 10;
    while (repeat--)
    {
      var angle = 45 + Random(90) + (180 * dir);

      var x = GetX() - Sin(angle, 500);
      var y = GetY() + Cos(angle, 500);

      CreateLightning(x, y, false);
    }
    if (actTime > 29)
    {
      CreateLightning(GetX(target), GetY(target), true, target);
    }
  }
}

func RotateForDeinstall()
{
  if (!deinstall) return;
  if (GetPhase() == 30) Deinstall();
 
  return(1);
}

func GetTargets()
{
  var a1 = Find_Distance(250);
  var a2 = Find_Hostile(GetController(bunker));
  var a3 = Find_OCF(OCF_Alive);
  var a4 = Find_NoContainer();
  var a5 = Find_PathFree();
  var a6 = Sort_Distance();

  return(FindObjects(a1, a2, a3, a4, a5, a6));
}

func CreateLightning(targetX, targetY, heighPower, target)
{ 
  var name = "EkeLightning";
  var dir = targetX > GetX();
  
  var x = -5 + (10 * dir);
  var y = -14;

  var angle = 90 + (180 * dir);
  var offset = 5;

  var size = 60 - (Random(25) * !heighPower);
  var color = RGBa(100, 100, 255, Random(255) * !heighPower);

  for (var i = 0; i < 100; i++)
  {   
    angle = GetNextAngle(x, y, targetX, targetY, angle, i);

    x -= Sin(angle, offset);
    y += Cos(angle, offset);

    var xDir = -Sin(angle, 10);
    var yDir = +Cos(angle, 10);

    if (!heighPower) { size--; offset = (size / 10) - 1; }

    var repeat = heighPower + 1;
    while (repeat--) CreateParticle(name, x, y, xDir, yDir, size, color);

    if (StrikeTarget(x, y, targetX, targetY, size, color, target)) break;
    if (GBackSemiSolid(x, y))                                      break;
    if (size < 30) 				                   break;
  }
}

func GetNextAngle(x, y, targetX, targetY, angle, length)
{
  var x1 = GetX() + x;
  var y1 = GetY() + y;
  var x2 = targetX;
  var y2 = targetY;

  var delta = Angle(x2, y2, x1, y1) - angle;
    
  angle += 50 - Random(100);

  if (length < Random(25)) // erstmal nach oben
  {
    if (angle < 090) angle = 090 + Random(50);
    if (angle > 270) angle = 270 - Random(50);
  }
  if (Abs(delta) > 45) angle += delta;
  
  return(angle);
}

func StrikeTarget(x, y, targetX, targetY, size, color, target)
{
  var x1 = GetX() + x;
  var y1 = GetY() + y;
  var x2 = targetX;
  var y2 = targetY;  

  var distance = Distance(x1, y1, x2, y2);
  if (distance > 2) return(false);
  
  CreateParticle("EkePulse", x, y, 0, 0, size + 100, color);
  CreateParticle("EkePulse", x, y, 0, 0, size + 050, color);
  CreateParticle("EkePulse", x, y, 0, 0, size + 025, color);

  if (target)
  {
    ParalyzeTarget(target);
    ScheduleCall(0, "ParalyzeTarget", 1, Random(10), target);
  }
  return(true);
}

func ParalyzeTarget(target)
{
  if (target)
  {
    SetXDir(0, target);
    SetYDir(0, target);    

    DoEnergy(-1, target);
  }
}