/*-- Line kit --*/

#strict 2

protected func Hit()
{
  Sound("RockHit*");
}

protected func Activate(clonk)
{
  [$TxtConnectLine$]
  // 
  var obj = FindObject2(Find_AtPoint(), Find_OCF(OCF_Fullcon), ConnectionCondition());
  // 
  if(!obj) return Message("$TxtNoNewLine$", clonk, GetLineName());
  // 
  //var line = FindObject2(Find_Action("Connect"),Find_ActionTarget(this));
  var line = FindObject(0, 0,0,0,0, 0, "Connect", this() );
  
  SetComDir(COMD_Stop, clonk);
  if(line) 
  {
  	if(obj == line->GetActionTarget(0) || obj == line->GetActionTarget(1))
  	{
  		line -> RemoveObject();
  		Sound("Connect");
  		return Message("$TxtLineRemoval$", clonk, GetLineName());
  	} 
  	else
  	{
  		line -> SetAction("Connect",obj,line->GetActionTarget(1));
  		Sound("Connect");
			Message("$TxtConnect$", clonk, GetLineName(), obj->GetName());
  		return RemoveObject();
  	}
  }
  else
  {
  	line = CreateObject(LineID(),0,0,NO_OWNER);
  	line -> SetAction("Connect",this,obj);
  	Sound("Connect");
  	return Message("$TxtConnect$", clonk, GetLineName(), obj->GetName());
  }
  return true;
}

protected func ContextFarRemoval(clonk)
{
	[$TxtFarRemoval$|Image=GLNE|Condition=GotLine]
	var line = FindObject(0, 0,0,0,0, 0, "Connect", this() );
	if (!line) return false;
	var obj = line->GetActionTarget(1);
	if (!obj) return false;
	line->RemoveObject();
	Sound("Connect");
  return Message("$TxtLineRemoval$", clonk, GetLineName());	
}

protected func GotLine()
{
	return !!FindObject(0, 0,0,0,0, 0, "Connect", this() );
}

public func ConnectionCondition() { return Find_OCF(OCF_LineConstruct); }
public func LineID () { return NONE; }
public func GetLineName() { return GetName(0, LineID()); }
public func IsLineKit() { return true; }
