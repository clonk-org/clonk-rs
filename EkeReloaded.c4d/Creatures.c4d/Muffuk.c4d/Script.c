/* Muffuk */

#strict

local size;
local alpha;
local steroids;
local heads;

func Initialize()
{
  size = 200;
  alpha = 255;
  steroids = 400;
  heads = CreateArray(4);

  SetAction("Grow");

  SetClrModulation(RGBa(255, 255, 255, alpha));
  AddEffect("Grow", this, 1, 1, this);

  // Enstehung dem Spiel(ziel) berichten
  GameCallEx("ReportCreation", this);
}

func FxGrowTimer()
{
  size++;
  alpha -= alpha > 0;
  
  var yOff = (1000 - size) * 26;
  SetObjDrawTransform(size, 0, 0, 0, size, yOff);
  
  // muss nicht sein, sieht aber geschmeidiger aus
  SetClrModulation(RGBa(255, 255, 255, alpha));

  if (size == 1000)
  {
    Awake();
    return(-1);
  }
}

func FxLifeCycleTimer()
{
  EkeVerifyEvilOwner();

  if (Random(ObjectCount(MF5B) * 10)) return;
  if (GetAction() ne "Seek")          return;

  var moff = CreateContents(MF5B);
  SetObjectOrder(moff, this);
  moff -> SetAction("Hatch", this);

  Exit(moff, 0, 10, 0, 0, -3);
  Sound("MU_Birth*");
}

func FxLifeCycleDamage(obj, fxNumber, value, cause)
{
  if (heads[0]) return;
  if (heads[1]) return;
  if (heads[2]) return;
  if (heads[3]) return;
  
  if (!Random(5)) Sound("MU_Scream*");
  return(value);
}

func FxLifeCycleEffect(wannabe)
{
  // Muffuk kann auch durch komische Zauber nicht anbrennen
  if (WildcardMatch(wannabe, "*Fire*")) return(-1);
}

func FxAttackTimer()
{
  var a0 = Find_Exclude();
  var a1 = Find_NoContainer();
  var a2 = Find_InRect(-200, -200, 400, 250);
  var a3 = Find_OCF(OCF_Alive);
  var a4 = Find_Hostile(GetOwner());
  var a5 = Find_PathFree();
  var a6 = Sort_Distance();

  var target = FindObject2(a0, a1, a2, a3, a4, a5, a6);
  if (target)
  {
    var b0 = GetAction() eq "Seek";
    var b1 = Abs(GetX(target) - GetX()) < 10;
    var b2 = Inside(GetY(target), GetY() - 30, GetY());

    if (b0 && b1 && b2)
    {
      Fling(target, (GetX() - GetX(target)) / 5, 1);
      SetAction("Close", target);
      SetObjectOrder(target, this);
    }
  }
  var newTimer = 35; if (target) newTimer = 1;
  ChangeEffect("Attack", this, 0, "Attack", newTimer);
}

func FxDigestTimer(victim)
{
  if (!victim)                   return(-1);
  if (!GetAlive(victim))         return(-1);
  if (Contained(victim) != this) return(-1);

  if (GetEnergy(victim) == 1)
  {
    while (Contents(0, victim)) Enter(this, Contents(0, victim));
    CreateContents(BONE, 0, 10);
  }  
  DoEnergy(-1, victim);
  steroids += 5;
  
  if (steroids > 100) if (!heads[0]) CreateHead(0, -08, 0, 135);
  if (steroids > 100) if (!heads[2]) CreateHead(2, +08, 0, 225);
  if (steroids > 100) if (!heads[1]) CreateHead(1, -12, 8, 120);
  if (steroids > 100) if (!heads[3]) CreateHead(3, +12, 8, 240);
}

func Awake()
{
  CreateHead(0, -08, 0, 135);
  CreateHead(1, -12, 8, 120);
  CreateHead(2, +08, 0, 225);
  CreateHead(3, +12, 8, 240);

  SetAction("Open");
  Sound("MU_Scream*");

  AddEffect("LifeCycle", this, 1, 35, this);
  AddEffect("Attack", this, 1, 35, this);
}

func CreateHead(i, x, y, a)
{
  heads[i] = CreateObject(TH5B, 0, 0, GetOwner());
  heads[i] -> Setup(this, x, y, a);

  steroids -= 100;
}

func Swallow()
{
  var victim = GetActionTarget();

  var b1 = Abs(GetX(victim) - GetX()) > 10;
  var b2 = Abs(GetY(victim) - GetY()) > 20;

  if (b1 || b2) return;
    
  Enter(this, victim);
  AddEffect("Digest", victim, 1, 100, this);
}

func Death()
{
  // wenn aus irgendeinem Grund noch Tentakel dran sind
  // z.B. wenn der Muffuk aus der Map rutscht
  if (heads[0]) heads[0] -> QuickRemove();
  if (heads[1]) heads[1] -> QuickRemove();
  if (heads[2]) heads[2] -> QuickRemove();
  if (heads[3]) heads[3] -> QuickRemove();

  for (var i = 0; i < 50; i++) 
  {
    var x = RandomX(-10, 10);
    var y = Random(25) - 5;    

    var xDir = RandomX(-50, 50);
    var yDir = RandomX(-50, 10);

    var s = Random(100) + 10;
    var v = Random(100) + 100;
    var c = RGB(200, v, v);
   
    CreateParticle("EkeStippelFlesh", x, y, xDir, yDir, s, c);
    CreateParticle("EkeBlood", x, y, xDir, yDir, s + 50);
  }
  Sound("MU_Die");
  AddEffect("FadeOut", this, 1, 1, this);

  // Tod dem Spiel(ziel) berichten
  GameCallEx("ReportHomicide", GetKiller(), GetOwner(), GetID(this));
}

func FxFadeOutTimer()
{
  alpha += 50;
  SetClrModulation(RGBa(255, 255, 255, alpha));

  if (alpha > 200)
  {
    Schedule("RemoveObject(0, 1)", 1);
    return(-1);
  }
}