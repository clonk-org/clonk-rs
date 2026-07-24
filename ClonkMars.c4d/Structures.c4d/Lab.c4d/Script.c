/*-- Lab --*/

#strict 2

#include B_80
#include OVDR
#include PWRC // ist ein Energieverbraucher
#include DACT //Damagecontrol
#include L_CA //Baugrafik

private func EnergyNeedOverlay() {
	return 3; // Overlay 1 und 2: Türsteuerung
}

/* Türsteuerung */

private func CanOpen() {
	return !IsResearching();
}

private func SoundOpenDoor() {
	Sound("Door_Metal");
}

private func SoundCloseDoor() {
	Sound("Door_Metal");
}

func Initialize() {
  SetAction("SiGreen");
  AddBuildingLight();
  
  upgrades = CreateArray();
  return(1);
}

protected func SiRed() {
	Sound("Lab_SiBeep_R");
}

/* Upgrades */

local upgrades;

public func AddUpgrade(id ID) {
	PushBack(ID, upgrades);
	ID -> ~UpgradeCompleted(GetOwner());
}

public func HasUpgrade(id ID) {
	return GetIndexOf(ID, upgrades) != -1;
}

global func UpgradeComplete(int iPlr, id ID) {
	return FindObject2(Find_ID(LABR), Find_Allied(iPlr), Find_Func("HasUpgrade", ID));
}

private func ApplyMenu(pClonk) {
	CreateMenu(LABR, pClonk, this, 0, "Keine anwendbaren Upgrades!");
	
	for(var ID in upgrades) {
		if(ID -> ~CanApply(this, pClonk))
			AddMenuItem(Format("%s anwenden", GetName(0, ID)), "ApplyUpgrade", ID, pClonk, 0, pClonk);
	}
}

protected func ApplyUpgrade(id ID, object pClonk) {
	if(ID -> ~CanApply(this, pClonk)) {
		ID -> ~Apply(this, pClonk);
		Sound("Connect");
	}
	else
		Sound("Error");
}

/* Forschung */

public func IsResearching() {
	for(var iNumber, i = 0; iNumber = GetEffect("Research", this, i); i++) {
		if(GetEffect(0, this, iNumber, 3))
			return true;
	}
}

private func ResearchMenu(object pClonk) {
  // Hauptmenü erzeugen
  CreateMenu(RSR1, pClonk, this);
  // alle gestarteten Forschungen, die noch nicht abgeschlossen wurden, durchgehen
  for(var iEffectNumber, i = 0; iEffectNumber = GetEffect("Research", this, i); i++) {
  	EffectVar(0, this, iEffectNumber) = pClonk;
  	AddMenuItem("$TxtContinuedevelopement$", "SelectResearch", EffectVar(1, this, iEffectNumber), pClonk, 0, iEffectNumber);
  }
  
  AddMenuItem("Neue Forschung", "NewResearch", GetID(), pClonk, 0, pClonk);
}

private func NewResearch(id idImage, object pClonk) {
	// Auswahlmenü erzeugen
	CreateMenu(idImage, pClonk, this, 0, "$TxtNothingelsetoresearc$");
	// Alle geladenen Baupläne überprüfen
	var i, idDef, iPlr = pClonk -> GetOwner();
	while(idDef = GetDefinition(i++, C4D_StaticBack)) {
		// Passende Kategorie und dem Spieler noch nicht bekannt
		if (!HasUpgrade(idDef) && idDef -> ~IsUpgrade(iPlr)) {
				AddMenuItem("$TxtResearchs$", "SelectResearch", idDef, pClonk, 0, pClonk);
		}
	}
}

protected func SelectResearch(id idDef, par) {
	var iEffectNumber, pClonk;
	if(GetType(par) == C4V_Int) {
		iEffectNumber = par;
		pClonk = EffectVar(0, this, iEffectNumber);
	}
	else
		pClonk = par;
	if(pClonk -> Contained() != this)
		return;
	StartResearch(idDef, pClonk, iEffectNumber);
}

protected func StartResearch(id idDef, object pClonk, int iEffectNumber) {
	// Spieler hat diesen Bauplan schon
	if(HasUpgrade(idDef)) {
		Message("$TxtAlreadyDeveloped$", this, GetName(0, idDef));
		Sound("Error");
		if(iEffectNumber)
			RemoveEffect(0, this, iEffectNumber, true);
		return; 
  }
  // Tür muss zu sein
  if(DoorActive) {
  	// gleich nochmal versuchen
  	ScheduleCall(this, "SelectResearch", 10, 0, idDef, pClonk, iEffectNumber);
  	return;
  }
  // Besitzer anpassen
  SetOwner(pClonk -> GetOwner());
	// Pause
	PauseAllResearch();
  // gibt es bereits einen entsprechenden Effekt?
  if(iEffectNumber)
  	ContinueResearch(iEffectNumber);
  // Effekt erstellen
  else
  	AddEffect("Research", this, 100, 15, this, GetID(), idDef, pClonk);
}

private func PauseAllResearch() {
  // alle gestarteten Forschungen pausieren
  for(var iNumber, i = 0; iNumber = GetEffect("Research", this, i); i++) {
  	PauseResearch(iNumber);
  }
  Sound("Lab_Research", 0, 0, 0, 0, -1);
  SetAction("SiGreen");
}

// es ist nicht sichergestellt, dass danach keine Forschung mehr läuft
// daher keine Änderung der Statuslichter
private func PauseResearch(int iNumber) {
	return ChangeEffect(0, this, iNumber, GetEffect(0, this, iNumber, 1), 0);
}

private func ContinueResearch(int iNumber) {
	Sound("Lab_Research", 0, 0, 0, 0, +1);
	SetAction("SiYellow");
	return ChangeEffect(0, this, iNumber, GetEffect(0, this, iNumber, 1), 15);
}

/* EffectVars:
 * 0: temporär gespeicherter Clonk
 * 1: ID, die erforscht wird
 * 2: benötigte Gesamtzeit
 * 3: bisherige Forschungsschritte
 * 4: Sound wird abgespielt?
 */

protected func FxResearchStart(object pTarget, int iEffectNumber, int iTemp, id idDef, object pClonk) {
	if(iTemp)
		return;
	EffectVar(1, pTarget, iEffectNumber) = idDef;
	var iTime = idDef -> ~GetResearchTime(this, pClonk);
	if(!iTime)
		iTime = RandomX(50, 200);
	EffectVar(2, pTarget, iEffectNumber) = iTime;
	Sound("Lab_Research", 0, 0, 0, 0, +1);
	SetAction("SiYellow");
	EffectVar(4, pTarget, iEffectNumber) = true;
}

protected func FxResearchEffect(string szNewEffectName, object pTarget, int iEffectNumber, int iNewEffectNumber, id idDef, object pClonk) {
	if(szNewEffectName == "Research" && idDef == EffectVar(1, pTarget, iEffectNumber)) {
		ContinueResearch(iEffectNumber);
		return -1;
	}
}

protected func FxResearchTimer(object pTarget, int iEffectNumber, int iEffectTime) {
	if(!ObjectCount2(Find_Container(this), Find_OCF(OCF_CrewMember)))
		return PauseAllResearch();
	if(!CheckPower(10)) {
		Sound("Lab_Research", 0, 0, 0, 0, -1);
		if(GetAction() != "SiRed")
			SetAction("SiRed");
		EffectVar(4, pTarget, iEffectNumber) = false;
		return;
	}
	if(!EffectVar(4, pTarget, iEffectNumber)) {
		SetAction("SiYellow");
		Sound("Lab_Research", 0, 0, 0, 0, +1);
		EffectVar(4, pTarget, iEffectNumber) = true;
	}
	var iProgress = ++EffectVar(3, pTarget, iEffectNumber);
	if(iProgress == EffectVar(2, pTarget, iEffectNumber))
		return -1;
}

protected func FxResearchStop(object pTarget, int iEffectNumber, int iReason, bool fTemp) {
	if(iReason)
		return;
	Sound("Lab_Research", 0, 0, 0, 0, -1);
	SetAction("SiGreen");
	Sound("ResearchDone");
	AddUpgrade(EffectVar(1, pTarget, iEffectNumber));
	// Nachricht ausgeben
	Message("$Txtsdeveloped$", this, GetName(0, EffectVar(1, pTarget, iEffectNumber)));
}

/* Steuerung */

public func ContainedUp(object pClonk) {
	[$TxtResearch$|Image=RSR1]
	if(IsResearching())
		PauseAllResearch();
	ResearchMenu(pClonk);
	return 1;
}

public func ContainedDig(object pClonk) {
	[Upgrades anwenden|Image=LABR]
	ApplyMenu(pClonk);
	return 1;
}

protected func ContextResearch(object pClonk) {
	[$TxtResearch$|Image=RSR1|Condition=IsNotResearching]
	ResearchMenu(pClonk);
	return 1;
}

private func IsNotResearching() {
	return !IsResearching();
}

protected func ContextCancelResearch(object pClonk) {
	[$TxtCancelresearch$|Image=RSR2|Condition=IsResearching]
	PauseAllResearch();
	return 1;
}

/* Wärme */

public func GetTemp() {
  return(150);
}

public func MaxDamage() { return 60; } //Maximaler Schaden

public func PipelineConnect() {
	return 1;
}

public func MarsResearch() {
	return true;
}

