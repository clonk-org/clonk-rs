/*-- HUD --*/

#strict 2

local Pointer;

func Initialize() {
  SetPosition(150, 105);
  SetVisibility(VIS_Owner);
  // Hide the base image.
  SetClrModulation(RGBa(0, 0, 0, 255));
  
  Pointer = CreateArray(2);
  return 1;
}

static const HUD_O2 = 1;
static const HUD_Fuel = 2;
static const HUD_Gencode = 3;
static const HUD_Temp = 4;
static const HUD_Log = 5;
static const HUD_ItemLog = 6;
static const HUD_EventLog = 7;
static const HUD_Pointer = 8; // +1
static const HUD_LandTemp = 8; // nicht für Overlays
static const HUD_ClonkTemp = 9; // sondern für UpdateHUD()

global func UpdateHUD(int iPlr, int iType, value) {
	var HUDs;
	if(iPlr == NO_OWNER)
		HUDs = FindObjects(Find_ID(MHUD));
	else if(iPlr < -1) // Spielernummer ab -2 für verbündete Spieler
		HUDs = FindObjects(Find_ID(MHUD), Find_Allied(Abs(iPlr) - 2));
	else
		HUDs = FindObjects(Find_ID(MHUD), Find_Owner(iPlr));
	if(!HUDs[0])
		return;
	for(var HUD in HUDs) {
		if(iType == HUD_O2)
			HUD -> UpdateO2(value);
		else if(iType == HUD_Fuel)
			HUD -> UpdateFuel(value);
		else if(iType == HUD_Gencode)
			HUD -> UpdateGencode(value);
		else if(iType == HUD_LandTemp)
			HUD -> UpdateTemperature(value);
		else if(iType == HUD_ClonkTemp)
			HUD -> UpdateClonkTemperature(value);
		else if(iType == HUD_ItemLog || iType == HUD_EventLog)
			HUD -> AddLog(value, iType);
	}
}

public func UpdateO2(int iO2) {
	if(iO2 < 0 || iO2 > 100) {
		DebugLog("ERROR: HUD: ungültige Sauerstofffüllung übergeben");
		return;
	}
	
	SetStillOverlayAction(Format("O2%d", 100-iO2), HUD_O2);
	return 1;
}

public func UpdateFuel(int iFuel) {
	if(iFuel < 0 || iFuel > 100) {
		DebugLog("ERROR: HUD: ungültige Treibstofffüllung übergeben");
		return;
	}
	
	SetStillOverlayAction(Format("Fuel%d", 100-iFuel), HUD_Fuel);
	return 1;
}

public func UpdateGencode(int iValue) {
	if(iValue < 0 || iValue > 100) {
		DebugLog("ERROR: HUD: ungültige Treibstofffüllung übergeben");
		return;
	}
	
	SetStillOverlayAction(Format("Gencode%d", 100-iValue), HUD_Gencode);
	return 1;
}

public func AddLog(val, int iType) {
	if(!val) {
		DebugLog("ERROR: HUD: kein Item/Event");
		return;
	}
	
	AddEffect("Log", this, 10, 1, this, 0, val, iType);
}

protected func FxLogStart(object pTarget, int iEffectNumber, bool fTemp, val, int iType) {
	if(fTemp)
		return;
	EffectVar(0, pTarget, iEffectNumber) = 255;
	EffectVar(1, pTarget, iEffectNumber) = CreateArray();
	EffectVar(2, pTarget, iEffectNumber) = iType;
	
	if(iType == HUD_EventLog)
		Sound("Special_Event", true, 0, 0, GetOwner()+1);
	
	DrawLogItem(iType, val, iEffectNumber);
}

protected func FxLogTimer(object pTarget, int iEffectNumber) {
	var alpha = EffectVar(0, pTarget, iEffectNumber);
	var waiting = EffectVar(1, pTarget, iEffectNumber);
	var type = EffectVar(2, pTarget, iEffectNumber);
	alpha--;
	
	if(!alpha && !waiting[0])
		return -1;
	
	if(alpha < 100 && waiting[0]) {
		DrawLogItem(type, waiting[0], iEffectNumber);
		DeleteArrayItem(0, waiting);
		alpha = 255;
		
		if(type == HUD_EventLog)
			Sound("Special_Event", true, 0, 0, GetOwner()+1);
	}
	
	SetClrModulation(RGBa(255, 255, 255, 255-alpha), this, type);
	EffectVar(0, pTarget, iEffectNumber) = alpha;
	EffectVar(1, pTarget, iEffectNumber) = waiting;
}

protected func FxLogEffect(string szEffectName, object pTarget, int iEffectNumber, int iNewEffectNumber, val, int iType) {
	if(szEffectName == "Log" && iType == EffectVar(2, pTarget, iEffectNumber))
		return -2;
}

protected func FxLogAdd(object pTarget, int iEffectNumber, string szNewEffectName, int iNewEffectTimer, val) {
	PushBack(val, EffectVar(1, pTarget, iEffectNumber));
}

protected func FxLogStop(object pTarget, int iEffectNumber, int iReason, bool fTemp) {
	if(fTemp)
		return;
	
	// Grafik zurücksetzen
	SetGraphics(0, this, 0, EffectVar(2, pTarget, iEffectNumber));
}

private func DrawLogItem(int iOverlay, val, int iEffectNumber) {
	if(iOverlay == HUD_ItemLog) {
		SetGraphics(0, this, val, iOverlay, GFXOV_MODE_IngamePicture);
		
		var iWidth, iHeight;
		iWidth = GetDefCoreVal("Picture", "DefCore", val, 2);
		iHeight = GetDefCoreVal("Picture", "DefCore", val, 3);
		
		// Größe des Bildes: angenommene 64x64
		// andere Grafikgrößen passen leider eher nicht (z.B. CNKT)
		SetObjDrawTransform(15 * 1000 / iWidth, 0, OverlayShiftX(iWidth) + 1000*(18), 0, 15 * 1000 / iHeight, OverlayShiftY(iHeight) + 7000, this, iOverlay);
	}
	else {
		SetGraphics(0, this, INFO, iOverlay, GFXOV_MODE_Action, val);
		SetObjDrawTransform(1000, 0, OverlayShiftX(28) + 1000*(45), 0, 1000, OverlayShiftY(28) + 7000, this, iOverlay);
	}
	
	
	
	
	return 1;
}

public func UpdateTemperature(int iTemp) {
	return MovePointer(0, iTemp);
}

public func UpdateClonkTemperature(int iTemp) {
	MovePointer(1, iTemp);
	if(iTemp > 0)
		iTemp = 0;
	SetViewportFreeze(GetOwner(), ChangeRange(MaxTemp-Abs(iTemp), 0, MaxTemp, 0, 255));
	return 1;
}

private func MovePointer(int iPointer, int iValue) {
	return AddEffect("MovePointer", this, 10, 1, this, 0, iPointer, iValue);
}

protected func FxMovePointerStart(object pTarget, int iEffectNumber, bool fTemp, int iPointer, int iValue) {
	if(!fTemp) {
		EffectVar(0, pTarget, iEffectNumber) = iPointer;
		EffectVar(1, pTarget, iEffectNumber) = iValue;
	}
}

protected func FxMovePointerTimer(object pTarget, int iEffectNumber) {
	var iPointer = EffectVar(0, pTarget, iEffectNumber), iValue = EffectVar(1, pTarget, iEffectNumber);
	
	// Änderungsrate abhängig von Unterschied => max: 10
	var change = (iValue - Pointer[iPointer]) / 100; //* (-2*(iValue < Pointer[iPointer])+1);
	
	DrawPointer(iPointer, Pointer[iPointer] + change);
	if(iValue == Pointer[iPointer])
		return -1;
}

protected func FxMovePointerEffect(string szEffectName, object pTarget, int iEffectNumber, int iNewEffectNumber, int iPointer) {
	if(szEffectName == "MovePointer" && iPointer == EffectVar(0, pTarget, iEffectNumber))
		return -2;
}

protected func FxMovePointerAdd(object pTarget, int iEffectNumber, string szNewEffectName, int iNewEffectTimer, int iPointer, int iValue) {
	EffectVar(1, pTarget, iEffectNumber) = iValue;
}

private func DrawPointer(int iPointer, int iValue) {
	if(Abs(iValue) > MaxTemp) {
		DebugLog("ERROR: HUD: falscher Wert für Pointer");
		return;
	}
	SetStillOverlayAction("Temperatur", HUD_Temp);
	var x, y;
	x = (13 + iPointer * 23) * 1000; // richtige Spalte
	y = 24000; // Position für 0
	
	var hgt = 27000; // Abstand zwischen 0 und Max/Min
	
	y -= iValue * hgt / MaxTemp;
	
	// Grafik setzen
	SetGraphics("Pointer", this, GetID(), HUD_Pointer + iPointer, GFXOV_MODE_Action, "Pointer");
	// an die richtige Stelle verschieben
	SetObjDrawTransform(1000, 0, x, 0, 1000, y, this, HUD_Pointer + iPointer);
	
	Pointer[iPointer] = iValue;
	
	return 1;
}
