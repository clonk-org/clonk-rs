/* Neomat */

#strict

func RejectCollect(stuff)
{
  var b1 = stuff == CC5B;
  var b2 = GetAction() ne "Work";

  if (b1 && b2) return(false);

  Sound("Error");
  return(true);
}
  
func Collection(cocoon)
{
  RemoveObject(cocoon);
  SetAction("Work");
  Sound("NM_Work");
}

func Finished()
{
  CreateObject(NH5B, 0, 13);
}