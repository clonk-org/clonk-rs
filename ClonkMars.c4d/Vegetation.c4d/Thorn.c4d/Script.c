/*-- Stalaktit --*/

#strict 2

local phase;

protected func Initialize() {
	SetAction("Hanging");
	SetPhase(phase=Random(4));
	SlopeRotation();

	ScheduleCall(this, "Grow", RandomX(100, 500));
}

private func Grow() {
	DoCon(1);
	if(GetCon() < 100) {
		Sound("Crack*");
		ScheduleCall(this, "Grow", 1);
	}
}

// Drehung nach Erdoberfläche
private func SlopeRotation() {
	var x_off = 18 * GetCon() / 100;
	var y_off = 15 * GetCon() / 100;
	var slope = GetSolidOffset(-x_off, y_off) - GetSolidOffset(x_off, y_off);
	SetR(slope);
}

protected func Damage() {
	if (GetDamage() > 10) {
		Destroy();
	}
}

private func Destroy() {
	var obj;
	Sound("RockBreak*");
	Sound("Blast2", 0, 0, 50);
	// links oben
	ObjectSetAction(obj=CreateObject(_STP, -4, -4, -1), "Exist");
	SetPhase(phase*2, obj);
	SetRDir(Random(20)-10, obj);
	SetSpeed(Random(30)-15, -Random(35)+10, obj);
	// rechts oben
	ObjectSetAction(obj=CreateObject(_STP, 3, -4, -1), "Exist");
	SetPhase(phase*2+1, obj);
	SetRDir(Random(20)-10, obj);
	SetSpeed(Random(30)-15, -Random(35)+10, obj);
	// unten
	ObjectSetAction(obj=CreateObject(_STP, 0, 12, -1), "LameExist");
	SetPhase(phase, obj);
	SetRDir(Random(20)-10, obj);
	SetSpeed(Random(10)-15, -Random(35)+10, obj);

	Regenerate();
	RemoveObject();
}

private func Regenerate() {
	// Find a new place to live.
	for(var i = 0; i < 10000; i++) {
		var wipf = PlaceAnimal(WIPF);
		var x = AbsX(wipf->GetX()), y = AbsY(wipf->GetY());
		wipf->RemoveObject();
		if(FindObject2(Find_AtPoint(x, y), Find_Category(C4D_Structure)))
			continue;
		var thorn = CreateObject(GetID(), x, y, NO_OWNER);
		thorn->SetCon(1);
		break;
	}
}
