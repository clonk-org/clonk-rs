/* PipeHead */

#strict 2

#appendto DPLI

public func Adjust()
{
  if (!GetActionTarget() || !pDerrick)
    return RemoveObject();
  SetPosition(GetX(pDerrick) + 13, GetY(pDerrick) + GetObjHeight(pDerrick) / 2);
}
