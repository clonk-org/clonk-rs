/* Kokon */

#strict

func Initialize()
{
  EkeVerifyEvilOwner();

  var maturity = 2000 + Random(1000);
  maturity += ObjectCount2(Find_ID(ST5B), Find_NoContainer());

  AddEffect("Birth", this, 1, maturity, this());

  // Enstehung dem Spiel(ziel) berichten
  GameCallEx("ReportCreation", this);
}

func FxBirthTimer()
{
  ChangeEffect("Birth", this, 0, "Birth", 35);  
  if (Contained() || GBackSemiSolid()) return;

  if (CreateMuffuk())     return;
  if (CreateMiffelTree()) return;

  CreateObject(ST5B, 0, 4, GetOwner());
  Sound("CC_Birth");
  RemoveObject();
}

func FxSellCheckTimer()
{
  var case;
  var caseCount;

  while (Contained(case))
  {
    case = Contained(case);
    caseCount++;
  }
  var baseOwner = GetBase(case);
  if (baseOwner >= 0)
  {
    while (--caseCount) Exit();

    DoWealth(baseOwner, GetValue());
    Sound("Cash");

    RemoveObject();
  }
}

func CreateMuffuk()
{
  if (Random(BoundBy(100 - ObjectCount(ST5B), 1, 50)))           return;
  if (FindObject2(Find_OCF(OCF_CrewMember), Find_Distance(500))) return;
  if (FindObject2(Find_ID(MU5B), Find_Distance(500)))            return;
  if (FindObject2(Find_ID(MR5B), Find_Distance(100)))            return;
  if (!Inside(GetX(), 100, LandscapeWidth() - 100))              return;

  for (var a = 120; a <= 240; a += 10)
  {
    var x = -Sin(a, 75);
    var y = +Cos(a, 75);

    if (!PathFree(GetX() + x, GetY() + y, GetX(), GetY())) return;
  }
  CreateObject(MU5B, 0, 5, GetOwner());

  Sound("CC_Birth");
  RemoveObject();
  return(true);
}

func CreateMiffelTree()
{
  if (!FindObject2(Find_ID(ST5B), Find_Distance(10), Find_NoContainer())) return;
  if (FindObject2(Find_Category(C4D_Structure), Find_Distance(50)))       return;
  if (FindObject2(Find_ID(MR5B), Find_Distance(100)))                     return;
  if (FindObject2(Find_ID(MU5B), Find_Distance(100)))                     return;
  if (!GBackSolid(0, 4))                                                  return;

  CreateConstruction(MR5B, 0, 5, GetOwner(), 10);
  
  Sound("CC_Birth");
  RemoveObject();
  return(true);
}

func Activate(clonk)
{
  DoDamage(100);
  return(1);
}

func Entrance()
{
  AddEffect("SellCheck", this, 1, 35, this());
  return(1);
}

func Hit()
{
  Sound("RockHit*");
  return(1);
}

func Damage(change, player)
{
  var toughness = 10 + Random(50);
  if (GetDamage() < toughness) return;

  for (var i = 0; i < 5; i++) 
  {
    var xDir = RandomX(-10, 10);
    var yDir = RandomX(-10, 10);

    var s = 10 + Random(50);
 
    CreateParticle("EkeStippelFlesh", 0, 0, xDir, yDir, s);
  }
  Sound("CC_Break");
  RemoveObject();

  // Zerstörung dem Spiel(ziel) berichten
  GameCallEx("ReportHomicide", player, GetOwner(), GetID(this));
}