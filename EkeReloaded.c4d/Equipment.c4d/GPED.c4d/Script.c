/* GPED */

#strict

local mode;   // mode = Blaster || Control || Biomap
local target; // ferngesteuertes Objekt
local index;  // Index des ferngesteuerten Objekts

func Initialize()
{
  mode = "Blaster";
  return(1);
}

func ControlLeft(object clonk)
{ 
  if (GetAction() ne "AirbikeFly") return(0);
  if (!target)                     return(0);

  SetPlrView(GetOwner(clonk), target);
  target -> ControlLeft(this());
  return(1);
}

func ControlLeftDouble(object clonk)
{ 
  if (GetAction() ne "AirbikeFly") return(0);
  if (!target)                     return(0);

  SetPlrView(GetOwner(clonk), target);
  target -> ControlLeftDouble(this());
  return(1);
}

func ControlRight(object clonk)
{ 
  if (GetAction() ne "AirbikeFly") return(0);
  if (!target)                     return(0);

  SetPlrView(GetOwner(clonk), target);
  target -> ControlRight(this());
  return(1);
}

func ControlRightDouble(object clonk)
{ 
  if (GetAction() ne "AirbikeFly") return(0);
  if (!target)                     return(0);

  SetPlrView(GetOwner(clonk), target);
  target -> ControlRightDouble(this());
  return(1);
}

func ControlUp(object clonk)
{ 
  if (GetAction() ne "AirbikeFly") return(0);
  if (!target)                     return(0);

  SetPlrView(GetOwner(clonk), target);
  target -> ControlUp(this());
  return(1);
}

func ControlDown(object clonk)
{ 
  if (GetAction() ne "AirbikeFly") return(0);
  if (!target)                     return(0);

  SetPlrView(GetOwner(clonk), target);
  target -> ControlDown(this());
  return(1);
}

func ControlThrow(object clonk)
{  
  if (mode != "Blaster") return(0);

  if (!target)
  {
    PlayerMessage(GetOwner(clonk), "$Selection$", this());
    Sound("Error");
    return(1);
  }
  SetPlrView(GetOwner(clonk), target);
  target -> BlowUp();
  return(1);
}

func ControlDig(object clonk)
{  
  if (mode == "Biomap") return(0);

  Departure(clonk);

  var criteria;
  
  if (mode == "Blaster") criteria = Find_Func("GPEDBlasterSelection", this(), 1);
  if (mode == "Control") criteria = Find_Func("GPEDControlSelection", this(), 1);

  var targets = FindObjects(criteria, Sort_Distance());
  
  if (index >= GetLength(targets)) index = 0;
  target = targets[index];
  index++;

  if (target)
  {
    SetComDir(COMD_Stop(), clonk);
    SetPlrView(GetOwner(clonk), target);

    if (mode == "Blaster") target -> GPEDBlasterSelection(this());
    if (mode == "Control") target -> GPEDControlSelection(this());
  }
  else
  {
    PlayerMessage(GetOwner(clonk), "$Existence$", this());
    Sound("Error");
  }
  return(1);
}

func ControlDigSingle()
{
  if (mode == "Biomap") return(0);
  return(1);
}

func ControlSpecial2(object clonk)
{
  Departure(clonk);

  var newMode;

  if (mode == "Blaster")
  {
    newMode = "Control";
  }
  if (mode == "Control")
  {
    newMode = "Biomap";
  }
  if (mode == "Biomap")
  {
    newMode = "Blaster";
  }
  mode = newMode;
  clonk -> ScrollHud(mode);
  return(1);
}

func Activate(object clonk)
{
  ControlDig(clonk);
  return(1);
}

func Departure(object clonk)
{
  target = 0;

  var owner = GetOwner(clonk);
  SetPlrView(owner, GetCursor(owner));

  // nach Airbike-Steuerung
  if (GetAction() ne "Walk") SetAction("Walk");
  return(1);
}

func Hit()
{
  Sound("MetalHit*");
  return(1);
}