/* Miffel */

#strict

static miffelCount;

local lastX;
local lastY;
local xDir;
local yDir;
local speed;
local angle;
local size;
local target;

func Initialize()
{
  if (FindObject(MR5B, 0, 1))
  {
    lastX = GetX();
    lastY = GetY();    

    size = 100;
    SetObjDrawTransform(size, 0, 0, 0, size);
  }
  else size = 1000;

  SetAction("Grow");
  SetDir(Random(2));

  // Enstehung dem Spiel(ziel) berichten
  GameCallEx("ReportCreation", this);

  miffelCount++;
}

func Destruction()
{
  miffelCount--;
}

func Death()
{
  var hsl = RGB2HSL(GetClrModulation());

  var h = (hsl >> 16) & 255;
  var s = (hsl >> 08) & 255;
  var l = (hsl >> 00) & 255;  

  for (var i = 0; i < (size / 50); i++) 
  {
    var c = HSL(h, s + Random(100), l - Random(50));
    var s = Random(size / 20);

    var s1 = (50 - s) / 5;
    var s2 = (50 - s) / 2;

    var x = RandomX(-s1, s1);
    var y = RandomX(-s1, s1) - 1;

    var xd = RandomX(-s2, s2);
    var yd = RandomX(-s2, s2);
   
    CreateParticle("EkeStippelFlesh", x, y, xd, yd, s, c);
  }
  Sound("MI_Die");
  Schedule("RemoveObject(0, 1)", 1);
  if (!l) CreateWings();

  // Tod dem Spiel(ziel) berichten
  var unborn = GetAction() eq "Grow";
  GameCallEx("ReportHomicide", GetKiller(), GetOwner(), GetID(this), unborn);
}

func CreateWings()
{
  if (GetAction() eq "Grow")   return;
  if (GetAction() eq "Unfold") return;

  var lWing = CreateObject(MW5B, +4 * GetDir() - 2, -2, -1);
  var rWing = CreateObject(MW5B, -2 * GetDir() + 1, -2, -1);

  lWing -> SetPhase(12);
  rWing -> SetPhase(15);

  SetDir(!GetDir(), lWing);
  SetDir(GetDir(), rWing);
}

func LifeCycle()
{
  EkeVerifyEvilOwner();

  if (OnFire()) Burning();

  if (GetAction() eq "Grow") return(Growing());
  if (GetAction() eq "Swim") return(Swimming());

  // nicht bewegen wenn Miffel festklemmt
  if (Stuck()) return(SetAction("Swim"));

  var b1 = GetAction() eq "Fly";
  var b2 = GetAction() eq "LoadedFly";
  var b3 = GetAction() eq "Turn";

  if (b1 || b2 || b3)
  {
    if (b1) HandleTarget();
    if (b2) LoadedFlying();

    AngleControl();
    SpeedControl();
  }
}

func FxFindTargetTimer()
{
  if (GetAction() eq "LoadedFly") return;
  if (GetAction() eq "Swim")      return;
  if (Stuck())                    return;

  var a0 = Find_Exclude();
  var a1 = Find_NoContainer();
  var a2 = Find_PathFree();

  var aa1 = Find_Distance(50 + Random(250));
  var aa2 = Find_OCF(OCF_Alive);
  var aa3 = Find_Hostile(GetOwner());

  var ac1 = Find_Distance(Random(50));
  var ac2 = Find_OCF(OCF_Collectible);
  var ac3 = Find_Or(Find_Category(C4D_Object), Find_ID(FLAG));

  var a3 = Find_Or(Find_And(aa1, aa2, aa3), Find_And(ac1, ac2, ac3));
  var a4 = Sort_Distance();

  target = FindObject2(a0, a1, a2, a3, a4);
}

func Burning()
{
  var hsl = RGB2HSL(GetClrModulation());

  var h = (hsl >> 16) & 255;
  var s = (hsl >> 8) & 255;

  SetClrModulation(HSL(h, s, 50 + GetEnergy() * 20));
}

func Growing()
{
  if (size == 1000)
  {
    SetAction("Unfold");
    Sound("MI_Unfold");

    AddEffect("FindTarget", this, 1, 35, this());
  }
  else
  {
    if ((GetX() != lastX) || (GetY() != lastY)) return(Kill());
    if (Random(miffelCount / 5)) return;

    size++;
    SetObjDrawTransform(size, 0, 0, 0, size);
  }
}

func Swimming()
{
  if (Stuck()) return;  

  // weiter wenn nicht mehr im Wasser
  if (GBackSemiSolid(0, -1)) return;

  SetPosition(GetX(), GetY() - 1);
  SetAction("Fly");
}

func HandleTarget()
{
  // nur wenn das Ziel in Reichweite und noch im Freien ist
  if (Distance(GetX(), GetY(), GetX(target), GetY(target) - 10) > 10) return;
  if (Contained(target))                                              return;

  // Lebewesen stechen
  if (GetOCF(target) & OCF_Alive())
  {
    var b1 = (GetX() < GetX(target)) && (GetDir() == DIR_Right());
    var b2 = (GetX() > GetX(target)) && (GetDir() == DIR_Left());

    if (b1 || b2)
    {
      if (GetAction(target) eq "AirbikeFly")
      {
        // Clonk nicht vom Airbike holen
        DoEnergy(-Random(10), target);
      }
      else Punch(target, Random(10));

      SetAction("Sting");
      Sound("MI_Sting");
    }
  }
  // Objekte klauen
  else SetAction("Load");
}

func LoadedFlying()
{
  var b0 = !target;
  var b1 = !Random(500);
  var b2 = Contained(target);
  var b3 = Distance(GetX(), GetY(), GetX(target), GetY(target) - 10) > 10;

  if (b0 || b1 || b2 || b3)
  {
    SetAction("Unload");
  }
  else
  {
    var height = GetDefCoreVal("Height", "DefCore", GetID(target));
    
    if (GetID(target) == FLAG)
    {
      target -> SetAction("Idle");
      height -= 5;
    }
    // Lasten nicht in den Boden drücken
    if (!GBackSolid(0, 8 + height))
    {
      SetPosition(GetX(), GetY() + 8 + (height / 2), target);
      SetYDir(-1, target); // der Schwerkraft entgegenwirken
    }
  }
}

func AngleControl()
{
  if (IsMoving() || (GetAction() eq "Sting"))
  {
    if (target)
    {
      var x = GetX(target);
      var y = GetY(target);

      angle = Angle(GetX(), GetY(), x, y - 15);

      if ((angle > 180) && (GetDir() == DIR_Right())) TurnLeft();
      if ((angle < 180) && (GetDir() == DIR_Left())) TurnRight();
    }
    else
    {
      angle += Random(50) - 25;
      angle -= 360 * (angle > 360);
      angle += 360 * (angle < 0);

      if ((xDir < -10) && (GetDir() == DIR_Right())) TurnLeft();
      if ((xDir > +10) && (GetDir() == DIR_Left())) TurnRight();
    }
  }
  else angle = Random(360);
}

func SpeedControl()
{  
  if ((GetAction() ne "Load") && (GetAction() ne "Sting"))
  {
    speed += Random(3) - 1;
    speed = Min(speed, 25);
    speed = Max(speed, 10 * !!target); // !! wegen obj -> bool

    xDir = BoundBy(xDir + Sin(angle, 5), -speed, speed);
    yDir = BoundBy(yDir - Cos(angle, 7), -speed, speed);

    // rückwärts nur langsam fliegen
    if (GetAction() eq "LoadedFly")
    {
      if ((xDir < -10) && (GetDir() == DIR_Right())) xDir = -5;
      if ((xDir > +10) && (GetDir() == DIR_Left()))  xDir = +5;
    }
    // nicht ins Wasser fliegen
    if ((yDir > 0) && GBackLiquid(0, 10 + yDir)) yDir -= yDir;
  }
  else
  {
    xDir = 0;
    yDir = 0;
  }
  SetXDir(xDir);
  SetYDir(yDir);
}

func IsMoving()
{
  var result = (GetX() != lastX) || (GetY() != lastY);
  
  lastX = GetX();
  lastY = GetY();

  return(result);
}

func TurnRight()
{
  if (Stuck() || GetAction() ne "Fly") return;

  SetDir(DIR_Right());
  if (!GBackSolid(-11, 0)) SetPosition(GetX() - 1, GetY());
}

func TurnLeft()
{
  if (Stuck() || GetAction() ne "Fly") return;

  SetDir(DIR_Left());
  if (!GBackSolid(11, 0)) SetPosition(GetX() + 1, GetY());
}