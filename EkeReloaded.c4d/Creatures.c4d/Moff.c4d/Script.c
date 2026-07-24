/* Moff */

#strict

local target;
local angle;
local dir;
local stuckTime;
local lastX;
local lastY;
local mutilated;

func Initialize()
{
  dir = 1;
  if (Random(2)) dir -= 2;

  SetAction("Walk");
  AddEffect("LifeCycle", this, 1, 25 + Random(25), this);

  // Enstehung dem Spiel(ziel) berichten
  GameCallEx("ReportCreation", this);
}

func FxLifeCycleTimer()
{
  EkeVerifyEvilOwner();

  if (OnFire())
  {
    DoEnergy(-1);
    Extinguish();
  }
  TargetControl();
  Movement();
  Attack();
}

func FxLifeCycleDamage(obj, fxNumber, value, cause)
{
  if (!mutilated && GetEnergy() < 75)
  {
    SetAction("Walk");
    SetPhysical("Walk", 40000, 2);

    SetGraphics("Mutilated");
    SetPosition(GetX() - 8 * dir, GetY());
    
    var leg = CreateObject(ML5B, 18 * dir, 13, GetOwner());
    SetObjDrawTransform(-1000 * dir, 0, 0, 0, 1000, 0, leg);
    
    SetYDir(-25, leg);
    SetRDir(Random(50) - 25, leg);
        
    var c = RGB(255, 200, 200);
    CastParticles("EkeStippelFlesh", 10, 50, 10 * dir, 8, 10, 50, c, c);

    mutilated = true;
  }
  if (target)
  {
    SetCommand(this(), "None");
    if (GetX() - GetX(target) > 0)
    {
      SetComDir(COMD_Right());
    }
    else
    {
      SetComDir(COMD_Left());
    }
  }
  if (!Random(2)) Sound("MF_Scream*");
  return(value);
}

func TargetControl()
{
  if (target)
  {
    var b1 = Random(5);
    var b2 = GetAlive(target);
    var b3 = !Contained(target);

    if (b1 && b2 && b3) return;
  }
  var a0 = Find_Exclude();
  var a1 = Find_NoContainer();
  var a2 = Find_Distance(100 + Random(250));
  var a3 = Find_OCF(OCF_Alive);
  var a4 = Find_Hostile(GetOwner());

  target = FindObject2(a0, a1, a2, a3, a4);
}

func Movement()
{
  if (target)
  {
    // Ziel verfolgen
    if (mutilated)
    {
      var x = GetX(target) + (dir * 10);
      var y = GetY(target); 

      SetCommand(this(), "MoveTo", 0, x, y);
    }
    else SetCommand(this(), "Follow", target);
  }
  else
  {
    // Laufrichtung wechseln
    SetCommand(this(), "None"); 
    ChangeComDir();
  }
  // den Rest kann der Moff mit 2 Beinen nicht mehr :'(
  if (mutilated) return;
  
  // Graben
  if (!Random(2) && stuckTime > 5)
  {
    if (target)
    {
      UpdateDir(target);
      Rotate();
    }
    SetAction("PrepareChop", this());
    stuckTime = 0;
  }
  // Springen
  var b0 = !InLiquid();
  var b1 = GetAction() eq "Walk";
  var b2 = Abs(angle) < 50000;
  var b3 = !Random(5) || stuckTime > 3;

  if (b0 && b1 && b2 && b3)
  {  
    // Priorität 3
    if (!Random(5))
    {
      dir *= -1;
      Rotate();
    }
    // Priorität 2
    if (target)
    {
      UpdateDir(target);
      Rotate();
    }
    // Priorität 1
    if (Abs(angle) > 10000) 
    {
      dir = angle / Abs(angle);
      Rotate();
    }
    SetAction("PrepareJump");
    stuckTime = 0;
  }
  // Moff steckt fest?
  if (lastX == GetX() && lastY == GetY()) stuckTime++;

  lastX = GetX();
  lastY = GetY();
}

func Attack()
{
  if ((GetProcedure() eq "FLIGHT") || InLiquid()) return;

  var a0 = Find_Exclude();
  var a1 = Find_OCF(OCF_Alive);
  var a2 = Find_Hostile(GetOwner());
  var a3 = Find_NoContainer();
  var a4 = Find_PathFree();
  var a5 = Find_Distance(25, -25 * dir * mutilated);

  var victim = FindObject2(a0, a1, a2, a3, a4, a5);
  if (victim)
  {
    if (!mutilated)
    {
      UpdateDir(victim);
      Rotate();
    }
    SetAction("PrepareChop", victim);
  } 
}

func UpdateAngle()
{
  var x1, y1, x2, y2;

  for (var a = 0; a < 180; a++)
  {
    x1 = -Sin(a, 25);
    y1 = +Cos(a, 25);

    if (!GBackSolid(x1, y1))
    {
      if (PathFree(GetX() + x1, GetY() + y1, GetX(), GetY())) break;
    }
  }
  for (var a = 360; a > 180; a--)
  {
    x2 = -Sin(a, 25);
    y2 = +Cos(a, 25);

    if (!GBackSolid(x2, y2))
    { 
      if (PathFree(GetX() + x2, GetY() + y2, GetX(), GetY())) break;
    }
  }
  var stuck = 12 - (y1 + y2) / 2 > 10;
  if (stuck)
  {
    SetXDir((y2 - y1) / Abs(y2 - y1) * 20);
    stuckTime++;
  }
  else angle = 90000 - Angle(x1, y1, x2, y2, 1000);
}

func ChangeComDir()
{
  if (Random(2))
  {
    SetComDir(COMD_Right());
  }
  else
  {
    SetComDir(COMD_Left());
  }
  if (!Random(10))
  {
    SetComDir(COMD_Stop());
  }
}

func UpdateDir(obj)
{
  dir = GetX() - GetX(obj);
    
  if (!dir) dir = 1;
  dir /= Abs(dir);
}

func Rotate()
{
  var sin = Sin(angle, 1000, 1000);
  var cos = Cos(angle, 1000, 1000);

  var xOff = 0;
  var yOff = GetVertex(0, true);

  var width  = cos * dir;
  var height = cos;

  var xSkew = sin;
  var ySkew = -sin * dir;

  var xAdj = (1000 - cos) * xOff - sin * yOff;
  var yAdj = (1000 - cos) * yOff + sin * xOff;

  SetObjDrawTransform(width, xSkew, xAdj, ySkew, height, yAdj);
}

func Walking()
{
  UpdateAngle();
  Rotate();
}

func StartJump()
{
  var xDir = Random(10) - 50; xDir *= dir;
  var yDir = Random(10) - 25;

  SetXDir(xDir); 
  SetYDir(yDir);
}

func Jumping()
{
  angle -= (angle - GetXDir() * 250) / 10;
  Rotate();
}

func Choping()
{
  var victim = GetActionTarget();
  if (victim == this())
  {
    var x = -Sin(angle + 90000 * dir, 25, 1000) + GetX();
    var y = +Cos(angle - 90000 * dir, 25, 1000) + GetY();

    DigFree(x, y, 10);
    Sound("MF_Dig");
  }
  else
  {
    var b0 = victim;
    var b1 = GetAlive(victim);
    var b2 = !Contained(victim);
    var b3 = ObjectDistance(victim) <= 25;

    if (b0 && b1 && b2 && b3)
    {
      DoEnergy(-25 + (15 * mutilated), victim);
      Fling(victim, dir, 1);
      Sound("MF_Chop");
    }
    else Sound("MF_Fail");
  }
}

func Hatched()
{
  var muffuk = GetActionTarget();
  SetObjectOrder(muffuk, this);
}

func Death()
{
  var leg1 = CreateObject(ML5B, +1 * dir, 13, GetOwner());
  var leg2 = CreateObject(ML5B, -5 * dir, 13, GetOwner());

  SetObjDrawTransform(0500 * dir, 0, 0, 0, 1000, 0, leg1);
  SetObjDrawTransform(1000 * dir, 0, 0, 0, 1000, 0, leg2);

  SetYDir(Random(10) - 25, leg1);
  SetYDir(Random(10) - 25, leg2);

  SetRDir(Random(50) - 25, leg1);
  SetRDir(Random(50) - 25, leg2);

  for (var i = 0; i < 10; i++) 
  {
    var x = Random(5) + 5 * dir;
    var y = Random(5) + 5;    

    var xDir = RandomX(-20, 20);
    var yDir = RandomX(-20, 20);

    var s = Random(50) + 10;
    var c = RGB(255, 200, 200);
   
    CreateParticle("EkeStippelFlesh", x, y, xDir, yDir, s, c);
  }
  Schedule("RemoveObject(0, 1)", 1);

  // Tod dem Spiel(ziel) berichten
  GameCallEx("ReportHomicide", GetKiller(), GetOwner(), GetID(this));
}