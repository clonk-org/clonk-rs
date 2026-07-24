/*-- Eingangssteuerung --*/

#strict 2

/* Dieses Objekt stellt die Grundfunktionalität für Gebäude
   mit Overlay-Tür zur Verfügung. */

local DoorActive, DoorClonk;

/* Die folgenden Funktionen nach Bedarf überladen */

private func ClonkOverlay() {
	return 1;
}

private func DoorOverlay() {
	return 2;
}

private func EnergyNeedOverlay() { // Overlay des Energiepfeils
	return 3;
}

private func ClonkCapacity() {
	return;
}

/* Ende der zu überladenden Funktionen */

protected func ActivateEntrance(object pObj) {
  if(CanOpen(pObj) && !DoorActive && !GetDefNoPushEnter(GetID(pObj)) && !Hostile(GetOwner(), pObj -> GetOwner())) {
  	SetOverlayAction("Door", DoorOverlay(), false, true, 0, "OpenEntrance");
  	SoundOpenDoor();
  	DoorActive = true;
  	if(pObj -> Contained()) {
			SetDoorClonk(pObj);
  	}  
  }
  return 1;
}

private func CanOpen(object pObj) {
	return ActIdle();
}
  
private func OpenEntrance()
{
  SetEntrance(1);
  ScheduleCall(this, "CloseEntrance", 20);
}

protected func CloseEntrance()
{
  SetEntrance(0);
  SetOverlayAction("Door", DoorOverlay(), true, true, 0, "DoorClosed");
  SoundCloseDoor();
}

private func DoorClosed() {
	ResetDoorClonk();
	DoorActive = false;
}

private func SoundOpenDoor()
{
  Sound("DoorOpen");
}

private func SoundCloseDoor()
{
  Sound("DoorClose");
}

protected func Completion()
{
	SetStillOverlayAction("Door0", DoorOverlay()); // geschlossene Tür
	return _inherited(...);
}

protected func Incineration() {
	RemoveOverlays();
	return _inherited(...);
}

public func Deconstruction() {
	RemoveOverlays();
	return _inherited(...);	
}

func Ejection(object pObj) {
	if(pObj == DoorClonk) {
		ResetDoorClonk();
	}
	if(GetEntrance()) {
		ClearScheduleCall(this, "CloseEntrance");
		ScheduleCall(this, "CloseEntrance", 20);
	}
	return _inherited(pObj, ...);
}

func Collection2(object pObj) {
	if(GetEntrance()) {
		if(pObj -> ~IsClonk() && (IsFullOfClonks(pObj) || Hostile(GetOwner(), pObj -> GetOwner()))) {
			//SetCommand(pObj, "Exit"); <- nicht möglich, da der Clonk damit erst kurz danach das Gebäude wieder verlässt
			if(GetDir() == DIR_Left)
				pObj -> Exit(pObj, GetDefCoreVal("Entrance", "DefCore", GetID(), 0) + GetDefCoreVal("Entrance", "DefCore", GetID(), 2) / 2, GetDefCoreVal("Entrance", "DefCore", GetID(), 1) + GetDefCoreVal("Entrance", "DefCore", GetID(), 3));
			else if(GetDir() == DIR_Right)
				pObj -> Exit(pObj, -GetDefCoreVal("Entrance", "DefCore", GetID(), 0) - GetDefCoreVal("Entrance", "DefCore", GetID(), 2) / 2, GetDefCoreVal("Entrance", "DefCore", GetID(), 1) + GetDefCoreVal("Entrance", "DefCore", GetID(), 3));
			pObj -> Sound("CommandFailure1");
			if(IsFullOfClonks(pObj))
				Message("$TxtFull$", pObj, GetName(), ClonkCapacity());
			return 1;
		}
		SetDoorClonk(pObj);
		ClearScheduleCall(this, "CloseEntrance");
		ScheduleCall(this, "CloseEntrance", 20);
	}
	return _inherited(pObj, ...);
}

public func IsFullOfClonks(object pObj) {
	return ClonkCapacity() && ClonkCapacity() <= ObjectCount2(Find_Container(this), Find_Exclude(pObj), Find_OCF(OCF_Alive), Find_Func("IsClonk"));
}

private func SetDoorClonk(object pObj) {
	if(!(pObj -> GetOCF() & OCF_CrewMember)) // nur Clonks, Fahrzeuge sind oft zu groß
		return;
	SetGraphics(0, this, pObj -> GetID(), ClonkOverlay(), GFXOV_MODE_Object, 0, 0, pObj);
	SetClrModulation(RGB(200, 200, 200), 0, ClonkOverlay());
	PlaceClonkOverlay(pObj);
	DoorClonk = pObj;
	return 1;
}

private func PlaceClonkOverlay(object pObj) {
	if(GetDir() == DIR_Left)
		SetObjDrawTransform(1000, 0, 1000*(GetDefCoreVal("Entrance", "DefCore", GetID(), 0) + GetDefCoreVal("Entrance", "DefCore", GetID(), 2) / 2), 0, 1000, 1000*(GetDefCoreVal("Entrance", "DefCore", GetID(), 1) + GetDefCoreVal("Entrance", "DefCore", GetID(), 3) + GetDefCoreVal("Offset", "DefCore", pObj -> GetID(), 1)), this, ClonkOverlay());
	else if(GetDir() == DIR_Right)
		SetObjDrawTransform(1000, 0, 1000*(-GetDefCoreVal("Entrance", "DefCore", GetID(), 0) - GetDefCoreVal("Entrance", "DefCore", GetID(), 2) / 2), 0, 1000, 1000*(GetDefCoreVal("Entrance", "DefCore", GetID(), 1) + GetDefCoreVal("Entrance", "DefCore", GetID(), 3) + GetDefCoreVal("Offset", "DefCore", pObj -> GetID(), 1)), this, ClonkOverlay());
}

private func ResetDoorClonk() {
	if(!IsFullOfClonks())
		SetGraphics(0, this, 0, ClonkOverlay());
}

private func RemoveOverlays() {
	SetGraphics(0, this, 0, DoorOverlay());
	SetGraphics(0, this, 0, ClonkOverlay());	
}
