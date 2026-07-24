#strict 2
#include TREE

local iPhase, onCeiling;

private func ReproductionAreaSize() { return 400; }
private func ReproductionRate()     { return 50; }
private func MaxTreeCount()         { return 15; }

protected func Initialize() {
	// Wir sind voll ausgewachsen und können gepflückt werden
	SetCategory(C4D_Object);
}

protected func Damage(iDamage) {
	ControlDigDouble();
}

public func Construction() {
	if(GetAction() == "Idle") {
		SetAction("Hang");		// zufällige Animationsphase
		iPhase = Random(4);
		// Zufällige Richtung
		if (Random(1))
			SetDir(DIR_Right);
	}
	if(Random(2)) {
		MoveDown();
	} else {
		MoveUp();
	}
	// Drehung nach Erdoberfläche
	var x_off = GetDefWidth(GetID()) / 2;
	var y_off = GetDefHeight(GetID()) / 2;
	var slope = 2 * GetSolidOffset(-x_off, y_off) - GetSolidOffset(x_off, y_off);
	if (onCeiling) slope = 180 - slope;
	SetR(slope); // passt besser
	SetPhase(iPhase);
	return 1;
}

private func MoveDown() {
	// Höhe anpassen
	while (!GBackSolid(0, 7) && GetY() < LandscapeHeight())
		SetPosition(GetX(), GetY() + 3);
	ScheduleCall(this, "MoveDown", 5);
}

// Tries to put the crystal on the ceiling, falling back to standard placement.
private func MoveUp() {
	var tunnel = Material("Tunnel");
	var y = -5;
	while (GetMaterial(0, y) == tunnel)
		y -= 5;
	if (GBackSolid(0, y)) {
		// Find the edge.
		while (GBackSolid(0, ++y));
		SetPosition(GetX(), GetY() + y + GetDefHeight(GetID()));
		onCeiling = true;
	} else {
		MoveDown();
	}
}

private func GetSolidOffset(int x, int y) {
	var i;
	var sign = 1;
	if (onCeiling) sign = -1;
	for (i = -20; GBackSolid(x, sign * (y - i)) && (i < 20); i++);
	return i;
}

public func Reproduction()
{
  // Ist noch Platz für einen Baum?
  var iSize = ReproductionAreaSize();
  var iOffset = iSize / -2;
  if (ObjectCount(GetID(), iOffset, iOffset, iSize, iSize)<MaxTreeCount()) {
    // OK, hin damit
    //var pMush = PlaceVegetation(GetID(), iOffset, iOffset, iSize, iSize, 10);
    Place(GetID(), Material("Tunnel"), iOffset, iOffset, iSize, iSize, 10);
    return 1;
  }
  // Kein Platz ;'(
  return;
}

private func Place(id ID, int mat, int x, int y, int wdt, int hgt, int growth) {
	var i = 50000;
	while(i--) {
		var px = x + Random(wdt), py = y + Random(hgt);
		if(GetMaterial(px, py) != mat)
			continue;
		
		return CreateConstruction(ID, px, py, NO_OWNER, growth);
	}
}

public func ContextChop(pClonk)		// per Kontextmenü pflücken
{
  [$TxtPick$|Image=MUSH|Condition=IsStanding]
  ControlDigDouble();
  return 1;
}

protected func RejectEntrance() {
  return IsStanding();
}

protected func ControlDigDouble() {
	if(IsStanding())
		Set(1);
	return;
}

protected func Activate(object pClonk) {
	[Kristall verarbeiten]
	pClonk -> CreateContents(CRYS);
	Sound("Crystal*");
	RemoveObject();
}

protected func Seed() {
  // re-seed
  if(!IsStanding() && !Contained() && GetYDir() <= 5 && !GetXDir())
  	Set(0);
  
  // steht der Kristall jetzt?
  if(IsStanding())
  	return _inherited(...);
}

public func IsStanding() { return GetAction() == "Hang"; }	// steht noch

public func IsTree() { return 0; } // kein Baum

/*func IsAlchemContainer() { return(true); }
func AlchemProcessTime() { return(80); }*/

protected func Hit() { Sound("Crystal*"); }

public func Set(int iPar) {
	if(!iPar) {
		SetAction("Hang");
		SetPhase(iPhase);
		return 1;
	}
	if(iPar == 1) {
		SetAction("Object");
		SetPhase(iPhase);
		return 1;
	}
	return;
}

// praktische Platzierung!

global func PlaceCrystals(int min, int max) {
	var i = RandomX(min, max);
	while(i--) {
		PlaceInMaterial(CRY2, Material("Tunnel"));
	}
}
