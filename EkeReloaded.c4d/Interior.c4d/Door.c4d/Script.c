/* Tür */

#strict

local code;

func Initialize()
{
  SetAction("TryToOpen");
  SetColorDw(RGB(100, 100, 100));  
}

func SetCode(newCode)
{
  code = newCode;
  SetColorDw(code);
}

func GetCode()
{
  return(code);
}

func IsOpen()
{
  SetSolidMask();
}

func IsClosed()
{
  SetSolidMask(0, 0, 11, 29);
}

func TryToOpen()
{
  for (var body in FindObjects(Find_InRect(-30, -10, 61, 40), 
                               Find_OCF(OCF_Alive)))
  {
    if (code)
    { 
      var keycard = FindContents(KC5B, body);

      if (!keycard)                         continue;
      if (keycard -> GetCode(this) != code) continue;
    }
    var bL = PathFree(GetX() - 6, GetY(), GetX(body), GetY(body));
    var bR = PathFree(GetX() + 6, GetY(), GetX(body), GetY(body));

    if (!bL && !bR) return;

    SetAction("Open");
    Sound("SteelGate2");
    break;
  }  
}

func TryToClose()
{
  var body = FindObject2(Find_InRect(-30, -10, 61, 40),
                         Find_OCF(OCF_Alive));
  if (body)
  {
    var bL = PathFree(GetX() - 6, GetY(), GetX(body), GetY(body));
    var bR = PathFree(GetX() + 6, GetY(), GetX(body), GetY(body));

    if (bL || bR) return;
  }
  SetAction("Close");
  Sound("SteelGate2");
}