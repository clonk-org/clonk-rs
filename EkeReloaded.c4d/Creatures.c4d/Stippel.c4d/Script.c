/* Stippel */

#strict

static stippelCount;

local lastAct;
local pregnancy;
local stuckTime;

func Initialize()
{
  // Farbe variiert leicht
  SetClrModulation(HSL(20, Random(255), 200 + Random(55)));

  SetAction("Walk");
  AddEffect("LifeCycle", this, 1, 10 + Random(40), this);

  // Enstehung dem Spiel(ziel) berichten
  GameCallEx("ReportCreation", this);

  stippelCount++;
}

func Destruction()
{
  stippelCount--;
}

func Death()
{
  for (var i = 0; i < 10; i++) 
  {
    var x = RandomX(-7, 7);
    var y = 5;    

    var xDir = RandomX(-20, 20);
    var yDir = -Random(20);

    var s = 10 + Random(50);
    var c = GetClrModulation(); 
   
    CreateParticle("EkeStippelFlesh", x, y, xDir, yDir, s, c);
  }
  Sound("ST_Die*");
  Schedule("RemoveObject(0, 1)", 1);

  // Tod dem Spiel(ziel) berichten
  GameCallEx("ReportHomicide", GetKiller(), GetOwner(), GetID(this));
}

// hässliches Rumgezappel verhindern
func ContactLeft()   { if (!GBackSolid(-8, 0)) SetPosition(GetX() + 1, GetY()); }
func ContactRight()  { if (!GBackSolid(8, 0))  SetPosition(GetX() - 1, GetY()); }
func ContactTop()    { if (!GBackSolid(0, -8)) SetVertex(0, 2, 0); }
func ContactBottom() {                         SetVertex(0, 2, 4); }

func FxLifeCycleTimer(target, effectNumber, effectTime)
{
  EkeVerifyEvilOwner();

  if (OnFire())
  {
    var hsl = RGB2HSL(GetClrModulation());

    var h = (hsl >> 16) & 255;
    var s = (hsl >> 8) & 255;

    SetClrModulation(HSL(h, s, GetEnergy() * 10));
  }
  PhysicalVariation();
  Pregnancy();
  Movement();
  Attack();
}

func PhysicalVariation()
{
  var speed = GetPhysical("Walk");

  speed += 500 - Random(1000);
  speed = BoundBy(speed, 5000, 15000);

  SetPhysical("Walk", speed, 2);
  SetPhysical("Scale", speed + 5000, 2);
  SetPhysical("Hangle", speed + 10000, 2);
}

func Pregnancy()
{
  pregnancy++;

  var cocoons = ObjectCount2(Find_ID(CC5B), Find_Distance(50));

  var b1 = pregnancy > 50 + Random(50) + stippelCount + (cocoons ** 2);
  var b2 = !FindObject2(Find_Hostile(GetOwner()), Find_Distance(100));
  var b3 = GetAction() eq "Walk";
  var b4 = !GBackSemiSolid();

  if (b1 && b2 && b3 && b4)
  {
    CreateObject(CC5B, 0, 8, GetOwner());
    Sound("ST_Cocoon*");
    pregnancy = 0;
  }
}

func Movement()
{
  // Richtung zufällig wechseln
  if (!Random(25) || !GetComDir())
  {
    var comDir;
    if (GetAction() ne "Scale")
    {
      comDir = COMD_Left();
      if (Random(2)) comDir = COMD_Right();
    }
    else
    {
      comDir = COMD_Up();
      if (Random(2)) comDir = COMD_Down();
    }
    SetComDir(comDir);
    HeaderControl();
  }
  // am Rand der Map umkehren
  if (GetX() < 10) SetComDir(COMD_Right());
  if (GetX() > LandscapeWidth() - 10) SetComDir(COMD_Left());

  // einfach so rumspringen
  if (!Random(25) && (GetProcedure() ne "FLIGHT") && !InLiquid())
  {
    // Default für Walk
    var xDir = Random(50) - 25;
    var yDir = Random(25) - 50;

    // bei Hangeln springen
    if (GetAction() eq "Hangle") yDir = 0;

    // beim Klettern springen
    var dir = 2 * GetDir() - 1;
    if (GetAction() eq "Scale") xDir = Abs(xDir) * -dir;

    SetXDir(xDir); 
    SetYDir(yDir);
 
    SetDir(xDir > 0);
    SetAction("Jump");
  }
  // feststeckende Stippel sterben irgendwann
  if (Stuck())
  {
    stuckTime++;
    if (stuckTime > 10)
    {
      DigFree(GetX(), GetY(), 10);
      Kill();
    }
  }
  else stuckTime = 0;
}

func Attack()
{
  if ((GetProcedure() eq "FLIGHT") || InLiquid()) return;
  
  var a0 = Find_Exclude();
  var a1 = Find_OCF(OCF_Alive);
  var a2 = Find_Hostile(GetOwner());
  var a3 = Find_NoContainer();
  var a4 = Find_PathFree();
  var a5 = Find_Distance(25 + Random(50));

  var victim = FindObject2(a0, a1, a2, a3, a4, a5);
  if (victim)
  {
    var xDir = GetX(victim) - GetX();
    var yDir = GetY(victim) - GetY() - 10;

    SetXDir(xDir); 
    SetYDir(yDir);
 
    SetDir(xDir > 0);

    SetAction("Bite", victim);
    Sound("ST_Bite*");
  }
}

func ComDirControl()
{
  if (GetAction() eq "Walk")
  {
    var b1 = GetDir() == DIR_Left();
    var b2 = GetDir() == DIR_Right();

    var b3 = lastAct == "scaleUp";
    var b4 = lastAct == "scaleDown";

    if ((b1 && b3) || (b2 && b4)) SetComDir(COMD_Left());
    if ((b2 && b3) || (b1 && b4)) SetComDir(COMD_Right());

    // damit der Stippel nicht runterrutscht
    if (b1 && b3) SetPosition(GetX() - 1, GetY());
    if (b2 && b3) SetPosition(GetX() + 1, GetY());
  }
  if (GetAction() eq "Scale")
  {
    var b1 = GetComDir() == COMD_Left();
    var b2 = GetComDir() == COMD_Right();

    var b3 = lastAct == "walk";
    var b4 = lastAct == "hangle";

    if ((b1 || b2) && b3) SetComDir(COMD_Up());
    if ((b1 || b2) && b4) SetComDir(COMD_Down());
  }
  if (GetAction() eq "Hangle")
  {
    var b1 = GetDir() == DIR_Left();
    var b2 = GetDir() == DIR_Right();
    
    var b3 = lastAct == "scaleUp";

    if (b1 && b3) SetComDir(COMD_Right());
    if (b2 && b3) SetComDir(COMD_Left());
  }
}

func VertexControl()
{ 
  if ((GetAction() eq "Walk") || (GetAction() eq "Jump"))
  {
    if (GetDir() == DIR_Left())
    {
      SetVertex(2, 2, 1);
      SetVertex(3, 2, 64);
    }
    if (GetDir() == DIR_Right())
    {
      SetVertex(2, 2, 64);
      SetVertex(3, 2, 2);  
    }
  }
  else
  {
    SetVertex(2, 2, 1);
    SetVertex(3, 2, 2);
  }
}

func HeaderControl()
{
  if ((GetAction() eq "Scale") && (GetComDir() == COMD_Down()))
  {
    // bergab kopfüber klettern
    SetObjDrawTransform(1000, 0, 0, 0, -1000);
  }
  else
  {
    // Tranformation zurücksetzen
    SetObjDrawTransform(1000, 0, 0, 0, 1000);
  }
}

func Walking()
{
  ComDirControl();
  VertexControl();
  HeaderControl();

  lastAct = "walk";
}

func Scaling()
{
  ComDirControl();
  VertexControl();
  HeaderControl();

  if (GetComDir() == COMD_Up())
  {
    lastAct = "scaleUp";
  }
  else
  {
    lastAct = "scaleDown";
  }
}

func Hangling()
{
  ComDirControl();
  VertexControl();
  HeaderControl();

  lastAct = "hangle";
}

func Jumping()
{
  VertexControl();
  HeaderControl();
}

func Biting()
{
  var victim = GetActionTarget();

  var b0 = victim;
  var b1 = GetActTime() < 15;
  var b2 = GetAlive(victim);
  var b3 = !Contained(victim);
  var b4 = ObjectDistance(victim) < 5;

  if (b0 && b1 && b2 && b3 && b4)
  {
    SetXDir(0);
    SetYDir(0);

    DoEnergy(-1, victim);
    Fling(victim, 4 * GetDir() -2, -1);
    
    // pro Angriff nur einmal zubeißen
    SetActionTargets();
  }
  VertexControl();
  HeaderControl();
}