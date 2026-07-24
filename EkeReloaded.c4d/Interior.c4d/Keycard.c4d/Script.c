/* Keycard */

#strict

local code;

func SetCode(newCode)
{
  code = newCode;
  SetColorDw(code);
}

func GetCode(door)
{
  if (door)
  {
    var doorCode = door -> GetCode();
    for (var childCard in FindObjects(Find_Container(this)))
    {
      var childCode = childCard -> GetCode();
      if (childCode == doorCode) return(childCode);
    }
  }
  return(code);
}

func Entrance(clonk)
{
  if (GetOCF(clonk) & OCF_CrewMember)
  {
    var keycard = FindObject2(Find_ID(KC5B),
                              Find_Container(clonk),
                              Find_Exclude());
    if (keycard)
    {
      var childCard;
      while (childCard = keycard -> Contents())
      {
        childCard -> Enter(this);
      }      
      keycard -> Enter(this);
    }
  }
}

func Activate(clonk)
{
  var text = Format("<c %x>•</c>", GetCode());
  for (var childCard in FindObjects(Find_Container(this)))
  {
    var childCode = childCard -> GetCode();
    text = Format("%s<c %x>•</c>", text, childCode);
  }
  PlayerMessage(GetOwner(clonk), text, this);
}