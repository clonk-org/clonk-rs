/*--Raumfahrer Clonk--*/

#strict 2
#include CLNK
#include JB4K
#include L_LP //Line pickup

static const SCNK_O2 = 2000; // f√ºr 3 Minuten= * 60 * 36
private func ReproductionTime() { return(300); }

local O2, O2Warning; // Sauerstoff in Prozent, akustische Warnung
local initialized;
local fReproduced; // ob der Clonk schon reproduziert wurde, nachdem er starb

protected func Initialize() {
	if(!initialized)
		initialized = true;
	else
		return;
	
	InitHUD();
	
	UpdateHUDValue(HUD_Gencode, 0);
	
	AddEffect("O2", this, 100, SCNK_O2 / 100, this);

	// Stirnlampe erstellen
	var pLamp = AddLightCone(500, RGBa(150, 150, 255, 20));
	pLamp -> ChangeSizeXY(500, 1000);
	pLamp -> LocalN("bAmbience") = true;
	AddEffect("Headlamp", pLamp, 100, 1, this);
	
	// Viewport
	if(!FindObject2(Find_ID(VWPT), Find_Owner(GetOwner()))) {
		CreateObject(VWPT, 0, 0, GetOwner());
	}
	AddEffect("ViewportCheck", this, 1, 0, this);

	return inherited(...);
}

/*Zellteilung*/

public func Birth() {
	Sound("Clonk_Birth");
	DoCon(-40);
	Schedule("DoCon(1)", 50, 40);
	UpdateHUD(GetOwner(), HUD_EventLog, "ClonkRepro");
}

public func IsReproducing() {
	return GetEffect("Reproduction", this);
}

public func IsReproduced() {
	return fReproduced;
}

public func Reproduced() {
	return fReproduced = true;
}

public func StartReproduction() {
	return AddEffect("Reproduction", this, 1, 40, this);
}

public func StopReproduction() {
	return RemoveEffect("Reproduction", this);
}

protected func FxReproductionTimer(object pTarget, int iEffectNumber, int iEffectTime) {
	if(!(Contained() -> CheckPower(15, false, true))) { // * 100
		EffectVar(0, pTarget, iEffectNumber) = 0;
		UpdateHUDValue(HUD_Gencode, 0);
		Sound("Error", 0, 0, 0, GetOwner()+1);
		PlayerMessage(GetOwner(), "$NoEnergy$", this);
		return -1;
	}
	
	// W‰hrend geforscht ist, teilen sich die Zellen nicht so gut
	// Der bisherige Teilungsstand wird jedoch unter Aufwendung von Energie behalten!
	if(Contained() -> IsResearching())
		return;
	
	UpdateHUDValue(HUD_Gencode, Min(++EffectVar(0, pTarget, iEffectNumber), 100));
	if(EffectVar(0, pTarget, iEffectNumber) >= 100 && !(Contained() -> ~IsFull())) {
	 var pClonk = CreateObject(SCNK, 0, 0, GetOwner());
	 pClonk -> Enter(Contained());
	 pClonk -> Birth();
	 
	 var death = FindObjects(Find_ID(SCNK), Find_Owner(GetOwner()), Find_Not(Find_OCF(OCF_Alive)), Find_Not(Find_Func("IsReproduced")), Sort_Random());
	 for(var pDeath in death) {
	 	pDeath -> Reproduced();
	  pClonk -> GrabObjectInfo(pDeath);	
	 	// ^- L2Einr¸cken und L2Tab
	 
	 	// Erfolgreich? Es kˆnnte schief gehen.
	 	if(pClonk == GetCrew(GetOwner()))
	 		break;
	 }
	 // keinen passenden Clonk gefunden, dann einen neuen erstellen
	 if(pClonk != GetCrew(GetOwner()))
	 	MakeCrewMember(pClonk, GetOwner());
	 
 	 // Rang ber¸cksichtigen (HP)
 	 pClonk -> DoEnergy(pClonk->GetPhysical("Energy")/1000 - pClonk->GetEnergy());
 	 
	 EffectVar(0, pTarget, iEffectNumber) = -1;
	}
}

protected func FxReproductionStop(object pTarget, int iEffectNumber, int iReason, bool fTemp) {
	if(!fTemp)
		UpdateHUDValue(HUD_Gencode, 0);
}

/* Itemlimit */
public func MaxContentsCount() { return 3; }

/* Sauerstoff */

protected func DoO2(int iChange) {
	var iPre = O2;
	O2 = BoundBy(O2 + iChange, 0, 100);
	return O2 - iPre;
}

global func HasO2() {
	if(this -> ~IsO2Producer())
		return this;
	else
		return FindObject2(Find_Func("IsO2Producer"), Find_Func("PipelineConnectedWith", this), Sort_Func("GetO2ProducerPriority"));
}

public func LowO2() {
	return O2Warning;
}

protected func FxO2Start(object pTarget, int iEffectNumber, bool fTemp) {
	if(fTemp)
		return;
	O2 = 100; // voller Sauerstoff am Anfang
	UpdateHUDValue(HUD_O2, O2);
}

// der Timerintervall ist so gew√§hlt, dass immer pro Zeiteinheit ein Prozentpunkt abgezogen wird
protected func FxO2Timer() {
	var pBuilding=0;
	// nicht, w‰hrend man in einem Geb‰ude ist, das Sauerstoff produziert
	if(Contained() && (pBuilding = Contained() -> HasO2()) || 
	   // ein Baum in der N‰he ist
	   ObjectCount2(Find_Distance(50), Find_Func("IsTree"), Find_Not(Find_Func("IsDeadTree"))) || 
	   // man einen schweren Anzug hat und ein verbundenes Pipekit
	   (HeavySuit() && ObjectCount2(Find_Container(this), Find_ID(PIKT), Find_Func("HasO2")))
	) {
		if(pBuilding) // Sauerstoff abziehen
			pBuilding->~DrainO2();
		// Sauerstoff nachladen
		O2 += 10;
		if(O2 > 100)
			O2 = 100;
		UpdateHUDValue(HUD_O2, O2);
	}
	else if(O2) {
		O2--;
		UpdateHUDValue(HUD_O2, O2);
	}
	else {
		DoEnergy(-20, this, false, FX_Call_EngAsphyxiation, GetOwner() + 1);
	}
	
	if(!O2Warning && O2 <= 30) {
		O2Warning = true;
		Sound("Warning_lowoxygen", true, 0, 0, GetOwner() + 1, +1);
	}
	else if(O2Warning && O2 > 30) {
		O2Warning = false;
		if(!ObjectCount2(Find_ID(GetID()), Find_OCF(OCF_Alive), Find_Owner(GetOwner()), Find_Func("LowO2")))
			Sound("Warning_lowoxygen", true, 0, 0, GetOwner() + 1, -1);
	}
}

protected func FxO2Stop(object pTarget, int iEffectNumber, int iReason, bool fTemp) {
	if(fTemp)
		return;
	if(O2Warning) {
		O2Warning = false;
		if(!ObjectCount2(Find_ID(GetID()), Find_OCF(OCF_Alive), Find_Owner(GetOwner()), Find_Func("LowO2")))
			Sound("Warning_lowoxygen", true, 0, 0, GetOwner() + 1, -1);
	}
}

protected func Entrance(object pContainer) {
	if(pContainer -> ~IsO2Producer()) {
		if(!O2)
			Sound("Breath");
	}
	return _inherited(pContainer, ...);
}

protected func Departure(object pContainer) {
	RemoveEffect("Reproduction", this);
	return _inherited(pContainer, ...);
}

/* HUD-Zeugs */

local HUD;

public func InitHUD() {
	if(!FindObject2(Find_Owner(GetOwner()), Find_ID(MHUD)))
		CreateObject(MHUD,-120,60, GetOwner());
	if(GetType(HUD) != C4V_Array)
		HUD = CreateHash();
	return 1;
}

public func UpdateHUDValue(int iType, int iValue) {
	HashPut(HUD, iType, iValue);
	if(GetCursor(GetOwner()) == this)
		UpdateHUD(GetOwner(), iType, iValue);
	return 1;
}

protected func CrewSelection(bool fDeselect) {
	if(!fDeselect) {
		var iter = HashIter(HUD);
		var node;
		while(node = HashIterNext(iter)) {
			UpdateHUD(GetOwner(), node[0], node[1]);
		}
	}
}

/* Spezialtaste: Inventarwechsel */ 
protected func ControlSpecial()  { 
	[$CtrlInventoryDesc$|Image=CXIV]
	// Inventory verschieben 
	return ShiftContents(0,0,0,1); 
}

// Reihenfolge im Kontextmen¸ ist umgekehrte Reihenfolge im Script
protected func ContextSizeSelection(object pClonk) {
	[$CtxSetResolution$|Image=VWPT]
	var iPlr = pClonk -> GetOwner();
	return FindObject2(Find_ID(VWPT), Find_Owner(iPlr)) -> SizeSelection();
}

public func ContextHome(pCaller)
{
  [$CtxHomeDesc$|Image=CXHM|Condition=HasBase]
  SetCommand(this, "Enter", FindBase(GetOwner(this)));
  return(1);
}

/* ‹berladung f¸r Deconstruct */
protected func ControlLeftDouble() {
	if (GetAction() == "Deconstruct" || GetAction() == "Repair")
		SetAction("Walk");
	return _inherited(...);
}

protected func ControlRightDouble() {
	if (GetAction() == "Deconstruct" || GetAction() == "Repair")
		SetAction("Walk");
	return _inherited(...);
}

protected func ContextDeconstruct(object clonk) {
	[$CtxDeconstructDesc$|Image=CRYC|Condition=AtBuildingToDeconstruct]
	var dummy = CreateObject(TIM1);
	SetGraphics("Chosen", dummy, MS4C, 1, GFXOV_MODE_Picture);
	CreateMenu(CXCN, this, this);
	AddMenuItem("$CtxDeconstructDesc$!", "StartDeconstruction", 0, this, 0, 0, "OK", 4, dummy);
	dummy -> RemoveObject();
}

protected func StartDeconstruction() {
	var struct = FindObject2(Find_AtObject(), Find_Category(C4D_Structure), Find_Allied(GetOwner()), Find_Not(Find_Func("NoDeconstruction")));
	if (!struct) {
		Message("$DeconstructNotFound$", this);
		return false;
	}
	if (GetAction() != "Walk") {
		Message("$DeconstructMustWalk$", this);
		return false;
	}
	SetAction("Deconstruct", struct);
}

protected func AtBuildingToDeconstruct() {
	return GetPhysical("CanConstruct") && FindObject2(Find_AtObject(), Find_Category(C4D_Structure), Find_Allied(GetOwner()));
}

protected func ContextRepair(object clonk) {
	[$CtxRepair$|Image=CXCN|Condition=AtBuildingToRepair]
	var struct = FindObject2(Find_AtObject(), Find_Category(C4D_Structure), Find_Allied(GetOwner()), Find_Func("GetDamage"), Find_Func("MaxDamage"));
	if (!struct) {
		Message("$NothingtoRepair$", this);
		return false;
	}
	if (GetAction() != "Walk") {
		Message("$Standing$", this);
		return false;
	}
	
	// Materialien einsammeln
	var needed = struct -> GetMissingComponents();
	for(var i = 0; i < GetLength(needed[0]); i++) {
		var ID = needed[0][i], num = needed[1][i], pObj;
		while((pObj = FindObject2(Find_Container(this), Find_ID(ID))) && num--) {
			struct -> DoRepairComponent(ID);
			pObj -> RemoveObject();
		}
	}
	SetAction("Repair", struct);
}

protected func AtBuildingToRepair() {
	return GetPhysical("CanConstruct") && FindObject2(Find_AtObject(), Find_Category(C4D_Structure), Find_Allied(GetOwner()), Find_Func("GetDamage"), Find_Func("MaxDamage"));
}

/* Stirnlampe */

protected func FxHeadlampTimer(object pTarget, int iEffectNumber) {
	if(!GetAlive()) {
		pTarget -> RemoveObject();
		return -1;
	}
	// Richtung an Clonk anpassen
  var rot = 5;
  var cx, cy;
  var phase = GetPhase();
  if (GetAction() == "Swim")
  {
    rot = 0;
    cx = -5;
    cy = 2;
  }
  else if (GetAction() == "Dig" || GetAction() == "Bridge")
  {
    if (phase > 7)  phase = 15-phase;
    rot = 15-phase*3;
    cx = -1+phase/3;
    cy = 6-phase*6/7;
  }
  else if (GetAction() == "Dive")
  {
    rot = BoundBy(phase*15, 5, 40+phase*5);
    cx = Min(0+phase*2, -3);
    cy = phase*13/7;
  }
  else if (GetAction() == "Tumble") {
  	rot = phase * (-20);
  	if(phase < 8) {
  		cx += phase;
  	}

  	if(phase < 13)
  		cy += phase;
  	else
  		cy += 20 - phase;
  }
  else if (GetAction() == "Build" || GetAction() == "Deconstruct") {
  	rot = 35;
  	cy += 5;
  }
  else if (GetAction() == "FlatUp") {
  	rot = (8 - phase) * 10;
  	cy += 10 - phase;
  }

  // VerrÔøΩckung der Helmlampe bei hoher Geschwindigkeit fixen
  cx += GetXDir() / 10;
  cy += GetYDir() / 10;

  
  // Richtung korrigieren
  pTarget -> ChangeOffset(cx-3, cy, true);
  if (GetDir() == DIR_Left)
  { 
    pTarget -> ChangeR(-90-rot);
    
  }
  else
  {
    pTarget -> ChangeR(90+rot);
  }
}

// hijacked from EKE >:D

private func Building()
{
  if (GetAction() == "Build")
  {
    // Funken und bl√§uliches Licht
    WeldingFX(20 * GetDir() - 10, 8);    
  }  
  return(1);
}

protected func Deconstructing()
{
	var struct = GetActionTarget();
	if (!struct) { //Schon abgerissen?
		SetAction("Walk");
		return;
	}
	Deconstruct(struct);
	// Funken und bl√§uliches Licht
	WeldingFX(20 * GetDir() - 10, 8);
	if (!struct) { //Jetzt abgerissen?
		SetAction("Walk");
		return;
	}	
}

private func Deconstruct(object struct)
{
	if (!struct) return;
	var cnmt = ObjectCount2(Find_ID(CNMT));
	if(cnmt) {
		var aIDs, aBefore, aAfter;
		var owner = GetOwner(struct); //Jetzt, da struct entfernt werden kann
		//IDs finden, aus denen das Geb‰ude zusammen gesetzt ist
		aIDs = CreateArray();
		for (var i = 0, component; component = GetComponent(0, i, struct); ++i) {
			aIDs[i] = component; 
		}
		aBefore = CreateArray(GetLength(aIDs));
		//Componenten vor der √Ñnderung finden
		for (var i = 0; i < GetLength(aIDs); ++i) {
			aBefore[i] = GetComponent(aIDs[i], 0, struct);
		}
	}
	
	var buildspeed = GetPhysical("CanConstruct");
	if (buildspeed == 1) buildspeed = 100;
	var mass = GetMass(0, GetID(struct));
	var change = Max(buildspeed * 30 / mass, 1);
	//Bei vollem erstmal Deconstruction aufrufen
	if (GetCon(struct) >= 100) {
		struct->~Deconstruction(this);
		for (line in FindObjects(Find_Action("Connect"), Find_ActionTargets(struct), Find_Not(Find_Func("ConnectedToLineKit")))) {
			LinePickup(/*dummy*/, line, true, true); //Leitung leise entfernen, und nicht immer aufnehmen
		}
	}
	DoCon(-change, struct);
	
	if(cnmt) {
		aAfter = CreateArray(GetLength(aIDs));
		//Componenten nach der ƒnderung finden
		if (struct) //aber nur, falls es das Geb√§ude noch gibt
		for (var i = 0; i < GetLength(aIDs); ++i) {
			aAfter[i] = GetComponent(aIDs[i], 0, struct);
		}
		
		//ƒnderung ausgeben
		for (var i = 0; i < GetLength(aIDs); ++i) {
			if (aBefore[i] > aAfter[i]) {
				CastObjects(aIDs[i], aBefore[i] - aAfter[i], 10);
			}
		}
	}
}

protected func Repairing() {
	var pObj = GetActionTarget();
	if(!pObj -> GetDamage())
		return SetAction("Walk");
	
	var buildspeed = GetPhysical("CanConstruct");
	if (buildspeed == 1) buildspeed = 100;
	var mass = GetMass(0, pObj -> GetID());
	var change = Max(buildspeed * 30 / mass, 1);
	
	// Mehr Material benˆtigt?
	if(!pObj -> Repair(change)) {
		// ausgeben!
		var needed = pObj -> GetMissingComponents();
		var output = "$NeededMaterials$";
		for(var i = 0; i < GetLength(needed[0]); i++) {
			var ID = needed[0][i], num = needed[1][i];
			output = Format("%s|%dx %s", output, num, GetName(0, ID));
		}
		Message(output, pObj);
		
		return SetAction("Walk");
	}
	
	// Funken und bl‰uliches Licht
	WeldingFX(20 * GetDir() - 10, 8);
}

protected func Recycling() {
	if(GetActTime() > 20) {
		GetActionTarget() -> Recycle();
		SetAction("Walk");
		return;
	}
	
	// Funken und bl‰uliches Licht
	WeldingFX(20 * GetDir() - 10, 8);
}

private func WeldingFX(xPos, yPos)
{
  CreateParticle("MSpark", xPos, yPos, RandomX(-15, 15), Random(15) * -1, 15, RGB(255, 255, 200));
  CreateParticle("MSpark", xPos, yPos, RandomX(-20, 20), Random(20) * -1, 15, RGB(255, 255, 170));
  CreateParticle("MSpark", xPos, yPos, RandomX(-25, 25), Random(25) * -1, 15, RGB(255, 255, 200));  

  CreateParticle("PSpark", xPos, yPos, RandomX(-1, 1), RandomX(-1, 1), RandomX(25, 50), RGB(Random(255), 255, 255));
  CreateParticle("PSpark", xPos, yPos, RandomX(-1, 1), RandomX(-1, 1), RandomX(100, 250), RGBa(Random(255), 255, 255, 150));
}

protected func Hit(int iXDir, int iYDir) {
	// nicht wenn wir schon tot sind
	if(!GetAlive())
		return;
	
	iXDir /= 10;
	iYDir /= 10;
	// Mit dem Satz des Herrn P. wird die Geschwindigkeit berechnet
	var iSpeed = Sqrt(iXDir**2 + iYDir**2);
	//Message("%d", this, iSpeed);
	if(iSpeed < 50)
		return;
	
	DoEnergy((50 - iSpeed) * 2, this, 0, FX_Call_EngObjHit);
	Sound("Hurt*");
	return 1;
}

protected func FxViewportCheckDamage(object pTarget, int iEffectNumber, int iDmgEngy, int iCause) {
	if(iDmgEngy < 0) {
		if(GetCursor(GetOwner()) == this)
			DoViewportBlood(GetOwner());
		else {
			// Cooldown
			if(FrameCounter() - EffectVar(0, pTarget, iEffectNumber) > 50) {
				UpdateHUD(GetOwner(), HUD_EventLog, "ClonkAttack");
				EffectVar(0, pTarget, iEffectNumber) = FrameCounter();
			}
		}
	}
	return iDmgEngy;
}

protected func Death() {
	for(var pObj in FindObjects(Find_ActionTarget(this), Find_Func("IsLight")))
		pObj -> RemoveObject();
	UpdateHUD(GetOwner(), HUD_EventLog, "ClonkDeath");

	// Call Clonk's Death() which will do the death notice.
	inherited(...);
	if(!GetCrew(GetOwner())) {
		// Respawn
		var dish = CreateObject(SATD, 0, 0, GetOwner());
		var clonk = CreateObject(GetID(), 0, 0, GetOwner());
		clonk->GrabObjectInfo(this);
		clonk->Enter(dish);
	}
	return 1;
}

/* Splitter-Anzeige */
protected func FindSplitter() {
	return FindObject2(Find_ID(SPTR), Find_AtPoint(), Find_Allied(GetOwner()));
}

protected func ContextSplitter() {
	[$CtxSplitter$|Image=SPTR|Condition=FindSplitter]
	var splitter = FindSplitter();
	if (splitter) {
		splitter -> ControlUp(this);
	}
}

/* Unterwasserlaufen */

protected func ContactBottom() {
	if(GetAction() == "Swim")
		SetAction("Walk");
}

/* Schwerer Anzug */

local heavysuit;

public func HeavySuit() { return heavysuit; }

public func UpgradeSuit() {
	// ein schwerer Anzug macht langsamer!
	SetPhysical("Walk", 60000, PHYS_Temporary); // 70000
	SetPhysical("Jump", 30000, PHYS_Temporary); // 40000
	SetPhysical("Scale", 27000, PHYS_Temporary); // 30000
	SetPhysical("Hangle", 27000, PHYS_Temporary); // 30000
	SetPhysical("Dig", 35000, PHYS_Temporary); // 40000
	return heavysuit = true;
}
