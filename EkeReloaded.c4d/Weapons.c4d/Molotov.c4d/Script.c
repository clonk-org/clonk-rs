/* Molotow */

#strict

func Damage()
{
  if (GetDamage() < 1) return(1);
  return(Hit());
}

func Hit()
{
  CastObjects(DFLM, 25, 25);
  Sound("MO_Break");
  RemoveObject();
  return(1);
}

func FX()
{
  if (Contained())           return;
  if (GBackSemiSolid(0, -5)) return;

  if (Abs(GetXDir()) + Abs(GetYDir()) > 10)
  {
    CreateParticle("PSpark", RandomX(-3, 3), RandomX(-8, -2), 0, 0, 150, RGBa(255, Random(2) * 100, 0, 120));
    CreateParticle("PSpark", RandomX(-3, 3), RandomX(-8, -2), 0, 0, 125, RGBa(255, Random(2) * 100, 0, 120));
    CreateParticle("PSpark", RandomX(-3, 3), RandomX(-8, -2), 0, 0, 100, RGBa(255, Random(2) * 100, 0, 120));
  }
  else
  {
    CreateParticle("PSpark", Random(2), RandomX(-6, -4), GetWind() / 10, -5, 40, RGBa(255, Random(2) * 100, 0, 120));
  }
  return(1);
}