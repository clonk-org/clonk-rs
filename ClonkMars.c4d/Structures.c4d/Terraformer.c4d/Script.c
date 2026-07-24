/*-- Terraformer --*/

#strict 2
#include B_40
#include PWRC
#include DACT //Damagecontrol
#include L_CA // Baugrafik

static const TRFM_RADIUS = 200; // Radius in Pixeln

protected func Initialize() {
	AddEffect("BlowAtmosphere", this, 1, 5, this);
	CreateDigger();
	fOn = true;
}

private func CreateDigger() {
	// Erde weggraben
	for(var iX = -TRFM_RADIUS; iX < TRFM_RADIUS; iX += 2) {
		var iY = 0;
		// an die Oberfläche versetzen
		while(GBackSolid(iX, iY))
			iY--;
		// direkt über dem Boden
		while(!GBackSolid(iX, iY + 2) && iY <= LandscapeHeight())
			iY++;
		
		// noch im Einflussbereich und innerhalb der Landschaft?
		if(Distance(iX, iY) > TRFM_RADIUS || iY > LandscapeHeight())
			continue;
		// Effekt erstellen
		AddEffect("DigEarth", this, 10, 10, this, 0, iX, iY);
	}
}

public func RemoveDigger() {
	var i = GetEffectCount("DigEarth", this);
	while(i--)
		RemoveEffect("DigEarth", this);
}

public func RecreateDigger() {
	RemoveDigger();
	CreateDigger();
}

// Sets x and y to the coordinates of a random digger.
public func GetRandomDiggerPos(&x, &y) {
	var digger = Random(GetEffectCount("DigEarth", this));
	x = EffectVar(0, this, digger);
	y = EffectVar(1, this, digger);
}

protected func Incineration() {
	RemoveDigger();
	return _inherited(...);
}

public func Deconstruction() {
	RemoveDigger();
	return _inherited(...);	
}

local iEnergy; // kleines Polster falls die Energie kurzzeitig nicht ausreicht (ab 10 reicht es nicht mehr)
local fTerraforming, fOn;

protected func Terraforming() { // TimerCall
	if(!(GetOCF() & OCF_Fullcon))
		return;
	if(!fOn || !CheckPower(30)) {
		iEnergy++;
		fTerraforming = false;
		return;
	}
		
	//play Animation
	SetAction("WindTurn");
	
	// Sound
	if(!Random(3))
		Sound("Terraformer_thunder*");
  	
	if(iEnergy)
		iEnergy--;
	  	
	fTerraforming = true;

	var treeCount = ObjectCount2(Find_Distance(TRFM_RADIUS), Find_Func("IsTree"));
	// Try to place two trees as soon as possible.
	if(treeCount < 2 || !Random(6) && treeCount < Random(10)) { // neue Bäume
		var v;
		if(!Random(2)) {
			v = PlaceVegetation(RandomTreeID(), TRFM_RADIUS / -2, TRFM_RADIUS / -2, TRFM_RADIUS, TRFM_RADIUS, 10);
			// nur ein Baum an einer Stelle
			if(v && ObjectCount2(Find_Distance(50, AbsX(GetX(v)), AbsY(GetY(v))), Find_Func("IsTree")) > 1)
				RemoveObject(v);
		}
		// erfolgreiche Platzierung nicht möglich?
		if(!v) {
			// zufälligen Digger wählen
			var x, y;
			GetRandomDiggerPos(x, y);
			// Kein Baum an der Stelle, aber LED in Reichweite?
			if(!FindObject2(Find_AtPoint(x, y), Find_Func("IsTree")) && FindObject2(Find_ID(LED_), Find_Distance(250, x, y), Find_Func("IsOn"))) {
				CreateConstruction(RandomTreeID(), x, y, NO_OWNER, 1);
			}
		}
	}

	// Place grass.
	if(ObjectCount2(Find_ID(GRAS), Find_Distance(TRFM_RADIUS)) < 10) {
		var x, y;
		GetRandomDiggerPos(x, y);
		if(!FindObject2(Find_AtPoint(x, y), Find_ID(GRAS))) {
			CreateConstruction(GRAS, x, y, NO_OWNER, 1);
		}
	}
	
	/* Temperatur anpassen */
	var iTemp = GetLandTemp(), iChange;
	if(iTemp > 0)
		iChange = -iTemp - 100 + iEnergy * 10;
	else
		iChange = -iTemp + 100 - iEnergy * 10;
	// bei dauerhaftem Betrieb funktionierts besser
	// -> 0
	DoLandTemp(-iTemp, 9);
}

public func NoEnergy() {
	return iEnergy > 10;
}

public func IsTerraforming() {
	return fTerraforming;
}

private func RandomTreeID() {
	var aTrees = [TRE5, TRE6, TRE7];
	return aTrees[Random(GetLength(aTrees))];
}

protected func FxDigEarthStart(object pTarget, int iEffectNumber, bool fTemp, int iX, int iY) {
	if(!fTemp) {
		EffectVar(0, pTarget, iEffectNumber) = iX;
		EffectVar(1, pTarget, iEffectNumber) = iY;
	}
}

protected func FxDigEarthTimer(object pTarget, int iEffectNumber) {
	var iX = EffectVar(0, pTarget, iEffectNumber), iY = EffectVar(1, pTarget, iEffectNumber);
	// Koordinaten anpassen
	if(GBackSolid(iX, iY)) {
		while(GBackSolid(iX, iY - 3))
			iY -= 1;
		if(pTarget -> IsTerraforming() && GBackSolid(iX, iY))
			DigFree(pTarget -> GetX() + iX, pTarget -> GetY() + iY, 2);
	}
	else if(!GBackSolid(iX, iY + 2)) {
		iY += 2;
	}
	
	// sind wir aus dem Einflussbereich gerutscht?
	if(pTarget -> Distance(iX, iY) > TRFM_RADIUS)
		return -1;
	
	EffectVar(1, pTarget, iEffectNumber) = iY;
}

public func ShowDigger() {
	var i = GetEffectCount("DigEarth", this);
	while(i--) {
		var iNum = GetEffect("DigEarth", this, i);
		CreateParticle("PSpark", EffectVar(0, this, iNum), EffectVar(1, this, iNum), 0, 0, 30, GetPlrColorDw(GetOwner()));
	}
}

protected func FxBlowAtmosphereTimer() {
	if(!IsTerraforming())
		return;
	
	// CO2
	CreateParticle("Smoke", 0, -18, 0, -20, 150, RGBa(255, 255, 255, 0));
	CreateParticle("Smoke", 0, -18, 0, -20, 50);
	
	// Regen
	for(var i = RandomX(2, 5); i; i--) {
		var iX = 0, iY = 0;
		while(!iX || Distance(iX, iY) > TRFM_RADIUS) {
			iX = Random(TRFM_RADIUS) - TRFM_RADIUS / 2;
			iY = RandomX(-TRFM_RADIUS, -TRFM_RADIUS / 2);
		}
		if(GBackSolid(iX, iY))
			continue;
		CreateParticle("Raindrop", iX, iY, RandomX(GetWind(), GetWind()*3), RandomX(100, 200), 5*64 + Random(32));
	}
}
  

/* Steuerung */

protected func ControlUp(){
	// Bereich anzeigen
	for(var i; i < 360; i++)
		CreateParticle("PSpark", Cos(i, TRFM_RADIUS), Sin(i, TRFM_RADIUS), 0, 0, 70, RGBa(255, 255, 255, 128));
}

protected func ControlDig() {
	ShowDigger();
}

protected func ControlThrow() {
	fOn = !fOn;
	fTerraforming = fOn;
}

protected func ContextRecreateDigger() {
	[Digger neu erstellen]
	RecreateDigger();
}

/* Erforschbar */

public func MarsResearch() {
	return true;
}

public func MaxDamage() { return 20; } //Maximaler Schaden
