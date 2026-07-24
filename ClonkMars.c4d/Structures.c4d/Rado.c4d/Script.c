/*-- Teleskop --*/

#strict 2
#include B_40
#include DACT //Damagecontrol
#include PWRC // Energieverbraucher
#include L_CA // Baugrafik

static const RADO_Distance = 250;

protected func Initialize(){
	SetAction("Turn");
}

protected func ControlLeft(object pClonk) {
	// not for JnR-Users
  if(GetPlrCoreJumpAndRunControl(GetController(pClonk)))
  	return;
  	
	AimLeft();
	return 1;
}


// Ändert den Zielwinkel nach links
func AimLeft()
{
	var iPhase = GetPhase() - 1;
	if(iPhase < 1) {
		Sound("Click");
		return;
	}
	SetPhase(iPhase);
}

protected func ControlRight(object pClonk) {
	// not for JnR-Users
  if(GetPlrCoreJumpAndRunControl(GetController(pClonk)))
  	return;
  	
	AimRight();
	return 1;
}

func AimRight()
{
	var iPhase = GetPhase() + 1;
	if(iPhase >= GetActMapVal("Length", "Turn", GetID())) {
		Sound("Click");
		return;
	}
	SetPhase(iPhase);
}

// for JnR-Users
public func ControlUpdate(object clonk, int comdir, bool dig, bool throw)
{
 // move aim to left
	if(comdir == COMD_Left)
		AimUpdate(clonk, COMD_Up, 5, "AimLeft"); // AimUpdate: System.c4g/JumpandRun.c
	// move aim to right
	else if(comdir == COMD_Right)
		AimUpdate(clonk, COMD_Down, 5, "AimRight");
	// stop changing the aim
	else
		AimUpdate(clonk, 0, 5, "");
	
	return(true);
}

protected func ControlDig() {
	DoBlast(true);
}

private func GetAngle() {
	var iAngle = [290,293,297,300,303,306,308,310,312,314,317,323,333,343,348,358,4,18,23,34,41,46,53,56,59,62,64,65,66,67,68][GetPhase() - 1];
	return iAngle;
}

private func AngleCriteria() {
	var iAngle = GetAngle();
	return Find_And(Find_Distance(RADO_Distance), Find_Angle(iAngle + 20, iAngle - 20), Find_PathFree());
}

local lastshot;

// power: wenn true, wird immer ein Blast ausgeführt, unabhängig davon, ob was gefunden wird
public func DoBlast(bool power) {
	// Cooldown
	if(FrameCounter() - lastshot < 18)
		return;

	// Nur, wenn fertig gebaut.
	if(GetCon() != 100)
		return;
	
	// Wenn generell ein Strahl abgeschossen werden soll
	if(power && !CheckPower(50)) {
		return;
	}
	
	// Nur bei Bedarf
	if(!power) {
		if(!ObjectCount2(Find_ID(METO), AngleCriteria())) {
			return;
		}
		var energyneed = GetEffect("EnergyNeed", this);
		if(!CheckPower(50)) {
			if(!energyneed) // nur, wenn davor kein Energiepfeil 
				UpdateHUD(-GetOwner() - 2, HUD_EventLog, "RadoEnergy");
			return;
		}
	}
	
	var iAngle = GetAngle();
	
	for(var pObj in FindObjects(Find_Or(Find_ID(METO), Find_OCF(OCF_Alive), Find_OCF(OCF_Collectible)), Find_NoContainer(), AngleCriteria())) {
		Fling(pObj, Sin(iAngle, 10), -Cos(iAngle, 10));
		pObj -> DoEnergy(-RandomX(5, 15));
		pObj -> Hit();
	}
	
	var pObj = CreateObject(RADE, 0, 0, GetOwner());
	pObj -> SetR(iAngle);
	pObj -> SetAction("Effect", this);
	
	Sound("RADO_Shoot");
	
	lastshot = FrameCounter();
	
	return 1;
}

public func MaxDamage() { return 32; } //Maximaler Schaden
public func MarsResearch() {return true;}
