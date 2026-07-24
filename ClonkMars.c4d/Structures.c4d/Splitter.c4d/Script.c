/*-- Splitter --*/

#strict 2
#include B_16
#include DACT //Damagecontrol

local pStation;

public func SetStation(object pObj) {
	return pStation = pObj;
}

public func GetStation() {
	return pStation;
}

protected func Damage() {
	if(!GetStation()) // unzerstörbar mit Station
		return inherited(...);
}

// Radius anzeigen
// Dies kann auch über das Kontextmenü des Spaceclonks aufgerufen werden, ohne das Gebäude anzufassen
public func ControlUp(object pClonk) {
	if(Hostile(GetOwner(), pClonk -> GetOwner())) {
		Sound("Error");
		return;
	}
	for(var i; i < 360; i++)
		CreateParticle("PSpark", Cos(i, TS_MaxDistance), Sin(i, TS_MaxDistance), 0, 0, 70, RGBa(255, 255, 0, 128));
	for(var pObj in FindTSNodes()) {
		var d = ObjectDistance(pObj);
		var a = Angle(AbsX(pObj->GetX()), AbsY(pObj->GetY()));
		CreateObject(LASR)->Set(a + 180, 5, d, 70);

	}
}

public func FindTSNodes() {
	return FindObjects(Find_Exclude(this), Find_ID(SPTR), Find_OCF(OCF_Fullcon), Find_Allied(GetOwner()), Find_Not(Find_Func("IsReserved")), Find_PathFree(this));
}

// Streckenreservierung

local fReserved;

// With upgraded splitters, there are no reservations.
public func IsReserved() {
	return !UpgradeComplete(GetOwner(), U_SR) && fReserved;
}

public func ToggleReservation() {
	return fReserved = !fReserved;
}

/* Wärme */

public func GetWarmth() {
  return(100);
}

public func MaxDamage() { return 5; } //Maximaler Schaden

// nicht freigraben
public func NoDigFree() { return true; }

/* Splitter-Fundament */

protected func Construction() {
	if(GetID() == BasementID())
		return;
	// Nur wenn noch kein Fundament vorhanden: Damit bei Überladung von
	// Fundamentobjekten nicht Fundamente mehrfach erstellt werden.
	basement = true;
	ScheduleCall(this, "CreateBasement", 1);
	return _inherited(...);
}

protected func CreateBasement() {
	var x = 0, y = 8 + GetDefHeight(SPTR) / 2;
	if(GetR()) {
		var radius = 4 + GetDefHeight(SPTR) / 2;//GetDefCoreVal("Offset", "DefCore", SPTR, 1);
		x = -Sin(GetR(), radius);
		y = Cos(GetR(), radius) + GetDefHeight(BasementID()) / 2;
	}
	
	basement = CreateObject(BasementID(), x, y, GetOwner());
	basement -> SetR(GetR());
}

/* Wir brauchen keinen Loreninhalt */

public func NoLorryEjection() {
	return true;
}
