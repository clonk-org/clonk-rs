/* Zelt */

#strict 2

public func Initialize()
{
  // das Szenario bestimmt den Look
  var suit = GameCall("GetSftSuit", this);
  if (suit) SetGraphics(suit);

  SetEntrance(1);
  return(1);
}

public func Unpack()
{
  SetEntrance(0);
  SetAction("Unpack");
  Sound("TN_Pack");
  return(1);
}

private func Unpacked()
{
  SetEntrance(1);
  
  var clonk = FindObject2(Find_OCF(OCF_CrewMember), 
                          Find_Action("Push"),
                          Find_ActionTarget(this()));
  
  AddCommand(clonk, "UnGrab");
  
  SetAction("Idle");
  return(1);
}

private func Packed()
{ 
  CreateObject(TB5B, 0, 15, GetOwner());
  RemoveObject();
  return(1);
}

public func CanPack()
{
  while(Contents())
  {
    if (GetOCF(Contents()) & OCF_Living && !GetAlive(Contents()))
    {
      // Leichen aus dem Zelt werfen
      Exit(Contents());
    }
    else return(false);
  }
  return(true);
}

protected func ControlDigDouble()
{
  if (GetAction() != "Idle") return(0);
  
  if (!CanPack())
  {
    Message("$Empty$", this());
    Sound("Error");
    return(0);
  }
  SetEntrance(0);

  // Attachte Objekte (Flaggen, etc.) loswerden
  for (var stuff in FindObjects(Find_ActionTarget(this())))
  {
    if (stuff -> GetProcedure() == "ATTACH") stuff -> SetAction("Idle");
  }
  SetAction("Pack");
  Sound("TN_Pack");
  return(1);
}

protected func SellTo()
{
  return(TB5B);
}