/*-- Station --*/

#strict 2
#include B_70
#include PWRC
#include DACT //Damagecontrol

static const TS_MaxDistance = 250;

local pSplitter;

protected func Initialize() {
	// Splitter erstellen
	pSplitter = CreateObject(SPTR, -27, 12 + GetDefHeight(SPTR) / 2, GetOwner());
	pSplitter -> SetStation(this);
  SetAction("Wait");
}

public func GetSplitter() {
	return pSplitter;
}

protected func Incineration() {
	pSplitter -> RemoveObject();
	return _inherited(...);
}

public func Deconstruction() {
	pSplitter -> RemoveObject();
	return _inherited(...);	
}

protected func ControlUp(object pClonk, bool fFast, object pObj2Beam){
	if(pSplitter -> IsReserved() || Hostile(GetOwner(), pClonk -> GetOwner())) {
		Sound("Error");
		return;
	}
	if(!pObj2Beam)
		pObj2Beam = pClonk;
	
	var Stations = FindStations();
	// Menü erstellen
	CreateMenu(GetID(), pClonk, this, 0, "$TxtNoTarget$");
	for(var pStation in Stations) {
		AddMenuItem(Format("Station #%d", ObjectNumber(pStation)), Format("Beam(%d, %d, %d)", ObjectNumber(pObj2Beam), ObjectNumber(pStation), fFast), GetID(), pClonk, 0, 0, "$TxtBeamstation$");
	}
	if(GetLength(Stations)) // nur, wenn das Menü auch Einträge hat
		MenuCheck(pClonk, Stations);
}

protected func ControlDig(object pClonk) {
	return ControlUp(pClonk, true);
}

protected func MenuCheck(object pClonk, array Stations, object pDummy) {
	if(pClonk -> GetMenu() != GetID()) {
		pDummy -> RemoveObject();
		if(GetCursor(pClonk -> GetOwner()))
			SetPlrView(pClonk -> GetOwner(), GetCursor(pClonk -> GetOwner()));
		return;
	}
	if(!pDummy) {
		pDummy = CreateObject(TIM1, 0, 0, pClonk -> GetOwner());
		pDummy -> SetVisibility(VIS_Owner);
		pDummy -> SetCategory(pDummy -> GetCategory() | C4D_Foreground);
	}
	var iSelection = pClonk -> GetMenuSelection();
	var pStation = Stations[iSelection];
	SetPlrView(pClonk -> GetOwner(), pStation);
	pDummy -> SetGraphics(0, pDummy, GetID(), 1, GFXOV_MODE_Object, 0, 0, pStation);
	pDummy -> SetClrModulation(RGBa(0, 255, 0, 128), 0, 1);
	pDummy -> SetPosition(pStation -> GetX(), pStation -> GetY());
	ScheduleCall(this, "MenuCheck", 5, 0, pClonk, Stations, pDummy);
}

protected func Beam(int iClonk, int iStation, bool fFast) {
	var pClonk = Object(iClonk), pStation = Object(iStation);
	var Way = FindWay(pStation -> GetSplitter(), fFast);
	if(!Way) {
		Sound("Error");
		return;
	}
	
	// Berechnung der benötigten Energie
	var iEnergy = 0, pObj, iDist;
	for(var pNode in Way) {
		// jeder Splitter kostet 5 Energie
		iEnergy += 5;
		// außerdem kommt ein Viertel der Entfernung zwischen den Splittern dazu
		if(pObj) {
			iDist = pNode -> ObjectDistance(pObj);
			if(iDist < TS_MaxDistance)
				iEnergy += iDist / 4;
			else // massiver Energieverbrauch bei langen Distanzen
				iEnergy += iDist / 2;
		}
		pObj = pNode;
	}

	// "Cheap Transmitter" upgrade
	if(UpgradeComplete(GetOwner(), U_CT)) {
		iEnergy /= 2;
	}
	Message("$TxtNeededEnergy$ %d kW", this, iEnergy);
	
	if(!CheckPower(iEnergy)) {
		Sound("Error");
		return;
	}
	for(var pObj in Way) {
		pObj -> ToggleReservation();
	}
	AddEffect("Beam", pClonk, 100, 50, 0, GetID(), Way);
  Sound("Transmitter_up");
  SetAction("Energize");
}

protected func FxBeamStart(object pTarget, int iEffectNumber, int iTemp, array aNodes) {
	if(iTemp)
		return;
	EffectVar(0, pTarget, iEffectNumber) = aNodes;
	pTarget -> Enter(aNodes[GetLength(aNodes)-1]);
}

protected func FxBeamTimer(object pTarget, int iEffectNumber) {
	var aNodes = EffectVar(0, pTarget, iEffectNumber), iLast = GetLength(aNodes) - 1;
	
	// fehlt der Splitter?
	if(!aNodes[iLast]) {
		pTarget -> Kill();
		pTarget -> RemoveObject();
		for(pNode in aNodes) {
			if(pNode)
				pNode -> ToggleReservation(); // alle freigeben
		}
		return -1;
	}
	
	// Laser
	var laser = LaserConnection(aNodes[iLast-1], aNodes[iLast]);
	laser->SetClrModulation(GetPlrColorDw(GetOwner(pTarget)));	

	pTarget -> Enter(aNodes[iLast-1]); // Target ins nächste Objekt verschieben
	aNodes[iLast] -> ToggleReservation(); // letzte Node wieder freigeben
	SetLength(aNodes, iLast); // letztes Element löschen
	if(iLast == 1) { // nur noch ein Element übrig?
		aNodes[0] -> ToggleReservation();
		pTarget -> Enter(aNodes[0] -> GetStation()); // ab in die Zielstation
		return -1;
	}
	EffectVar(0, pTarget, iEffectNumber) = aNodes;
}

protected func FxBeamStop(object pTarget, int iEffectNumber, int iReason, bool fTemp) {
	if(fTemp)
		return;
	// Don't stop beaming when the Clonk suffocates.
	if(iReason == 4)
		return -1;

	var pStation = pTarget -> Contained();
	pStation -> Sound("Transmitter_down");
	pStation -> SetAction("Energize");
	pTarget -> Exit(0, 18, 21);
}

// Stationen im Netz finden
private func FindStations() {
	var Stations = CreateArray();
	var OpenList = [GetSplitter()], ClosedList = CreateArray();
	var pStation;
	while(GetLength(OpenList)) {
		var CurrentNode = OpenList[GetLength(OpenList) - 1];
		ClosedList[GetLength(ClosedList)] = CurrentNode;
		SetLength(OpenList, GetLength(OpenList) - 1);
		
		// evtl. in die Stationsliste eintragen
		if(CurrentNode != GetSplitter() && (pStation = CurrentNode -> GetStation()) && GetIndexOf(pStation, Stations) == -1)
			Stations[GetLength(Stations)] = pStation;
		
		for(var Successor in CurrentNode -> FindTSNodes()) {
			// Steht bereits in der ClosedList?
			if(GetIndexOf(Successor, ClosedList) != -1)
				continue;
			OpenList[GetLength(OpenList)] = Successor;
		}
	}
	return Stations;
}

// sucht einen Weg vom aktuellen Objekt zu pObj
private func FindWay(object pObj, bool fFast) {
	// OpenList und ClosedList initialisieren
	var OpenList = CreateHash(), ClosedList = CreateHash();
	HashPut(OpenList, GetSplitter(), []);
	// solange es noch Einträge in der OpenList hat
	while(HashSize(OpenList)) {
		var CurrentNode;
		// Paar mit dem kleinsten f finden...
		var iter = HashIter(OpenList), node, min = 0; // wichtig: var setzt die Variablen nicht zurück!
		while(node = HashIterNext(iter)) {
			if(!min || min > node[1][0]) {
				min = node[1][0];
				CurrentNode = node[0];
			}
		}
		// und aus der OpenList entfernen und in die ClosedList setzen
		HashPut(ClosedList, CurrentNode, HashGet(OpenList, CurrentNode));
		HashErase(OpenList, CurrentNode);
		
		if(CurrentNode == pObj) {
			var LastObject = pObj, Way = [pObj];
			while(LastObject != GetSplitter()) {
				Way[GetLength(Way)] = LastObject = HashGet(ClosedList, LastObject)[1];
			}
			return Way; // Weg gefunden!
		}
		
		// Nodes suchen
		ExpandNode(CurrentNode, OpenList, ClosedList, pObj, fFast);
	}
	return; // kein Weg gefunden
}

private func ExpandNode(object CurrentNode, &OpenList, array ClosedList, object pTarget, bool fFast) {
	for(var Successor in CurrentNode -> FindTSNodes()) {
		// Steht bereits in der ClosedList?
		if(HashContains(ClosedList, Successor))
			continue;
		if(!HashContains(ClosedList, CurrentNode))
			continue; 
		
		var iDist = CurrentNode -> ObjectDistance(Successor);
		// Wenn die Distanz größer als TS_MaxDistance ist, zählt sie doppelt (-> optimaler Energieverbrauch)
		if(!fFast && iDist < TS_MaxDistance)
			iDist /= 2;
		// alter Wert + neuer Wert + Kosten pro Splitter
		var f = HashGet(ClosedList, CurrentNode)[0] + iDist + 10;
		
		if(fFast)
			f = HashGet(ClosedList, CurrentNode)[0] + iDist + Successor -> ObjectDistance(pTarget);
		// Steht bereits in der OpenList und hat größeres f?
		if(GetType(HashGet(OpenList, Successor)) == C4V_Array && HashGet(OpenList, Successor)[0] < f)
			continue; // nichts tun
		
		HashPut(OpenList, Successor, [f, CurrentNode]);
	}
}

/* Wärme */

public func GetWarmth() {
  return(130);
}

public func MaxDamage() { return 40; } //Maximaler Schaden

/* Erforschbar */

public func MarsResearch() {
	return true;
}

/* Wir brauchen keinen Loreninhalt */

public func NoLorryEjection() {
	return true;
}
