/*-- Platform --*/

#strict 2
#include B_60
#include DACT //Damagecontrol
#include L_CA

local pCaps;

public func MaxDamage() { return 35; } //Maximaler Schaden
protected func Initialize(){SetAction("Wait");}

public func PortActive(){
  //ausgeführt sobald Kapsel kommt
  if(GetAction() != "Active") SetAction("Active");
  }
  
public func PortWait(){
  if(GetAction() != "Wait") SetAction("Wait");
  }
  
public func Occupy(object pOcc){
  if(pOcc && pCaps) Log("Internal Error: Capsule requesting occupation for occupated Port.");
  if(!pOcc) {Log("Internal Error: Request for Occupation without object."); return 0;}
  pCaps = pOcc;
  }
  
public func IsFree(){
	if(pCaps) return false;
	return true;
}

protected func CheckCapsule() {
	if(!pCaps || !pCaps->Flying()) SetAction("Wait");
}

public func MarsResearch() {
	return true;
}
