/* Bombenkiste */

#strict

func Activate(object clonk)
{
  for (var i = 0; i < 3; i++) 
  {
    CreateContents(OB5B, clonk);
  }
  ShiftContents(clonk, true, OB5B);
  Sound("SF_Process");
  RemoveObject();
  return(1);
}

func Hit()
{
  Sound("RockHit*");
  return(1);
}