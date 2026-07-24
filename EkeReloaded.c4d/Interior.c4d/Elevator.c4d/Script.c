/* Fahrstuhl */

#strict

local pickup;

func Initialize()
{
  SetAction("Wait");
}

func Waiting()
{
  var clonk = FindObject2(Find_OCF(OCF_CrewMember),
              Find_InRect(-40, -5000, 80, 10000),
              Sort_Distance());
  
  if (!clonk)                          return;
  if (Abs(GetY() - GetY(clonk)) < 30)  return;
  if (GetComDir(clonk) != COMD_Stop()) return;

  var offset = 20 - 40 * (GetY(clonk) < GetY());

  if (!PathFree(GetX(), GetY() + offset, GetX(), GetY(clonk)))  return; // Brücke dazw.
  if (!PathFree(GetX(), GetY(clonk), GetX(clonk), GetY(clonk))) return; // Tür dazw.

  pickup = GetY(clonk);
  if (pickup < GetY())
  {
    Start(COMD_Up());
  }
  else
  {
    Start(COMD_Down());
  }
}

func Moving()
{
  if (pickup)
  {  
    var delta = pickup - GetY();    

    SetYDir(BoundBy(delta, -50, +50));
    if (Abs(delta) < 30) pickup = 0;
    
    return; 
  }
  if (!FindObject2(Find_Action("Push"), Find_ActionTarget(this())))
  {
    // nächste Tür suchen
    var offset = 10; if (GetYDir() > 0) offset -= 31;
    var door = FindObject2(Find_ID(DO5B), Find_InRect(-30, GetYDir() + offset, 61, 10));
    if (door)
    {    
      var delta = GetY(door) - GetY() + 1;
      if (!delta) return(Stop());
      
      //Message("@%d", this(), delta);

      // Fahrstuhl bremsen
      SetYDir(GetYDir() - delta);
    }
  }
}

func Start(cDir)
{
  SetSolidMask(0, 80, 49, 40);

  SetAction("Move");
  SetComDir(cDir);
}

func Stop()
{
  SetSolidMask(0, 40, 49, 40);

  SetAction("Wait");
  SetComDir(COMD_Stop());
  SetYDir(0);
}

func ControlUp()         { return(Start(COMD_Up()));   }
func ControlDownSingle() { return(Start(COMD_Down())); }
func ControlLeft()       { return(Stop());             }
func ControlRight()      { return(Stop());             }
func ContactTop()        { return(Stop());             }
func ContactBottom()
{
  SetPosition(GetX(), GetY() - 1);
  return(Stop());
}