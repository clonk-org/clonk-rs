/*-- Materiallabor --*/

#strict 2

#include B_95
#include OVDR
#include PWRC // ist ein Energieverbraucher
#include DACT //Damagecontrol
#include L_CA // Baugrafik

static const UNIT_metal_earth_cost = 2;
static const UNIT_metal_ashes_cost = 3;
static const UNIT_plastic_cost = 350;
public func GetTemp() { if(GetAction() == "Produce") return 500; return 250;}

local slot;

protected func Initialize() {
	//Briefkasten
	slot = CreateObject(CUNT, -41, 26, GetOwner());
	slot->SetUnit(this);
	slot->SetObjectOrder(this);
	return _inherited(...);
}

protected func Destruction() {
	slot->RemoveObject();
	return _inherited(...);
}

public func Deconstruction() {
	slot->RemoveObject();
}

private func CanOpen() {
	return GetAction() == "Idle";
}

private func SoundOpenDoor() {
	Sound("Door_Metal");
}

private func SoundCloseDoor() {
	Sound("Door_Metal");
}

private func EnergyNeedOverlay() {
	return 3; // Overlay 1 und 2: Türsteuerung
}

protected func ContainedUp(object pClonk) {
	return ProduceMenu(pClonk);
}

protected func ContainedDig(object pClonk) {
	return ProduceMenu(pClonk, true);
}

protected func ProduceMenu(object pClonk, fShowAll) {
	if(GetAction() != "Idle") {
		CreateMenu(GetID(), pClonk, this);
		if(fContinue)
			AddMenuItem("$TxtCancel$", "fContinue = false;", RSR2, pClonk, 0, 0, "$TxtCancelFull$");
		if(GetAction() != "Wait")
			AddMenuItem("$TxtCancelNow$", "Cancel", RSR2, pClonk, 0, 0, "$TxtCancelNow$");
		return 1;
	}
	
	CreateMenu(GetID(), pClonk, this, C4MN_Extra_Components, "$TxtNothingProducible$");
	if(fShowAll || ObjectCount2(Find_Container(this), Find_ID(ORE1)) || 
	               ObjectCount2(Find_Container(this), Find_ID(ERTH)) >= UNIT_metal_earth_cost ||
	               ObjectCount2(Find_Container(this), Find_ID(ASHS)) >= UNIT_metal_ashes_cost
	) {
		AddMenuItem("Metall", "ProduceMetal", METL, pClonk);
	}
	if(fShowAll || ObjectCount2(Find_ID(OILT), Find_Func("PipelineConnectedWith", this))) {
		AddMenuItem("Plastik", "ProducePlastic", PSTC, pClonk);
	}
	
	// Lore ebenfalls hardcoded..
	if(fShowAll || CheckComponents(LORY)) {
		AddMenuItem(GetName(0, LORY), "ProduceLorry", LORY, pClonk);
	}
	
	// Universalherstellung von Objekten
	var i, idDef, iPlr = pClonk -> GetOwner();
	while(idDef = GetPlrKnowledge(iPlr, 0, i++, C4D_Object)) {
		if(fShowAll || CheckComponents(idDef))
			AddMenuItem(GetName(0, idDef), "Produce", idDef, pClonk);
	}
	return 1;
}

/* Produce */

static const UNIT_door_retry_wait = 3;

local idProduce, fContinue;

protected func ProduceMetal(idItem, iParameter, fRight) {
	if (DoorActive) {
		// Try again soon.
		ScheduleCall(this, "ProduceMetal", UNIT_door_retry_wait, 0, idItem, iParameter, fRight);
	}
	if (GetAction() != "Idle" || DoorActive || !FindObject2(Find_Container(this), Find_OCF(OCF_CrewMember)))
		return fContinue = false;
	if(!CheckPower(100, true)) {
		if(fRight)
			fContinue = true;
		if(fContinue) {
			SetAction("Wait");
			idProduce = METL;
		}
		return;
	}
	
	var pOre = FindObject2(Find_Container(this), Find_ID(ORE1));
	var aEarth = FindObjects(Find_Container(this), Find_ID(ERTH));
	var aAshes = FindObjects(Find_Container(this), Find_ID(ASHS));
	if(pOre || GetLength(aEarth) >= UNIT_metal_earth_cost || GetLength(aAshes) >= UNIT_metal_ashes_cost) {
		if (pOre)
			pOre -> RemoveObject();
		else if(GetLength(aEarth) >= UNIT_metal_earth_cost) {
			for (var i = 0; i < UNIT_metal_earth_cost; ++i)
				aEarth[i] -> RemoveObject();
		}
		else {
			for (var i = 0; i < UNIT_metal_ashes_cost; ++i)
				aAshes[i] -> RemoveObject();
		}
		CheckPower(100);
		SetAction("Produce");
		idProduce = METL;
		if(fRight)
			fContinue = true;
	}
	else {
		Sound("Error");
		fContinue = false;
	}
}

protected func ProducePlastic(idItem, iParameter, fRight) {
	if (DoorActive) {
		// Try again soon.
		ScheduleCall(this, "ProducePlastic", UNIT_door_retry_wait, 0, idItem, iParameter, fRight);
		return;
	}
	if(GetAction() != "Idle" || DoorActive || !FindObject2(Find_Container(this), Find_OCF(OCF_CrewMember)))
		return fContinue = false;
	if(!CheckPower(100, true)) {
		if(fRight)
			fContinue = true;
		if(fContinue) {
			SetAction("Wait");
			idProduce = PSTC;
		}
		return;
	}
	
	for(var pTank in FindObjects(Find_ID(OILT), Find_Func("PipelineConnectedWith", this))) {
		if(pTank -> GetFill() >= UNIT_plastic_cost) {
			CheckPower(100);
			pTank -> DoFill(-UNIT_plastic_cost);
			SetAction("Produce2");
			idProduce = PSTC;
			if(fRight)
				fContinue = true;
			return 1;
		}
	}
	
	Sound("Error");
	Message("$TxtNotEnoughOil$", this);
	fContinue = false;
}

protected func ProduceLorry(idItem, iParameter, fRight) {
	if (DoorActive) {
		// Try again soon.
		ScheduleCall(this, "ProduceLorry", UNIT_door_retry_wait, 0, idItem, iParameter, fRight);
		return;
	}
	if (GetAction() != "Idle" || DoorActive || !FindObject2(Find_Container(this), Find_OCF(OCF_CrewMember)))
		return fContinue = false;
	if(!CheckPower(100, true)) {
		if(fRight)
			fContinue = true;
		if(fContinue) {
			SetAction("Wait");
			idProduce = idItem;
		}
		return;
	}
	
	if(CheckComponents(idItem, true)) {
		CheckPower(100);
		SetAction("Produce");
		idProduce = idItem;
		if(fRight)
			fContinue = true;
	}
	else {
		Sound("Error");
		fContinue = false;
	}
}

private func CheckComponents(id idItem, bool fRemove) {
	var i, ID, iAmount, aObjs = [];
	while(ID = GetComponent(0, i++, 0, idItem)) {
		iAmount = GetComponent(ID, 0, 0, idItem);
		for(var pObj in FindObjects(Find_Container(this), Find_ID(ID))) {
			if(fRemove)
				aObjs[GetLength(aObjs)] = pObj;
			if(!--iAmount)
				break;
		}
		if(iAmount)
			return;
	}
	if(fRemove) {
		for(var pObj in aObjs)
			pObj -> RemoveObject();
	}
	
	return 1;
}

// Universal-Herstellen
protected func Produce(idItem, iParameter, fRight) {
	if (GetAction() != "Idle" || DoorActive || !FindObject2(Find_Container(this), Find_OCF(OCF_CrewMember)))
		return fContinue = false;
	if(!CheckPower(100, true)) {
		if(fRight)
			fContinue = true;
		if(fContinue) {
			SetAction("Wait");
			idProduce = idItem;
		}
		return;
	}
	
	if(CheckComponents(idItem, true)) {
		CheckPower(100);
		// anderer Sound bei Erdherstellung
		if(idItem == ERTH)
			SetAction("Crush");
		else
			SetAction("Produce2");
		idProduce = idItem;
		if(fRight)
			fContinue = true;
	}
	else {
		Sound("Error");
		fContinue = false;
	}
}

protected func MeltCrushing() {
	Sound("Melt_Crushing");
}

protected func Finish() {
	CreateContents(idProduce);
	
	// HUD-Update für alle Spieler, die einen Clonk hier drin haben
	var aPlr = CreateArray(GetPlayerCount()), iPlr;
	for(var pClonk in FindObjects(Find_Container(this), Find_OCF(OCF_CrewMember))) {
		iPlr = pClonk -> GetOwner();
		if(!aPlr[iPlr]) {
			UpdateHUD(iPlr, HUD_ItemLog, idProduce);
			aPlr[iPlr] = true;
		}
	}
	
	Sound("Pshshsh");
	Continue();
}

protected func Continue() {
	if(fContinue) {
		if(idProduce == METL)
			ProduceMetal(0, 0, true);
		else if(idProduce == PSTC)
			ProducePlastic(0, 0, true);
		else if(idProduce == LORY)
			ProduceLorry(idProduce, 0, true);
		else
			Produce(idProduce, 0, true);
	}
}

protected func Cancel() {
	SetAction("Idle");
	Sound("Pshshsh");
	fContinue = false;
}
  
/*Smoke*/

protected func Smoking() {
	if(!Random(2))
		Smoke(17, Random(3) - 23, Random(9) + 20);
	if(!Random(4))
		Smoke(17, Random(3) - 19, Random(5) + 20);
	if(GetActTime() >= 550)
		SetAction("Wait");
}

public func PipelineConnect() {
	return 1;
}

public func MaxDamage() { return 50; } //Maximaler Schaden

/* Kontext */

private func IsComplete()
{
  return (GetCon() >= 100); 
}

/* Erforschbar */

public func MarsResearch() {
	return true;
}

private func ClonkCapacity() {
	return 2;
}
