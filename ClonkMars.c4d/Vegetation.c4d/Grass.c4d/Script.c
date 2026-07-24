/*-- Grass --*/

#strict 2

#include TRE5

// Grass does not produce O2.
public func IsTree() { return false; }

protected func Construction() { 
	// Zufällige Form  
	SetAction("Grass");
	SetPhase(Random(2));
	if (!Random(20)) SetPhase(2);
	// Zufällige Richtung
	if (Random(2)) SetDir(DIR_Right);
	Grow();

	inherited(...);

	ScheduleCall(this, "MoveToGround", 1);
	// Gras bleibt hinter Bäumen
	MoveBehindTrees();
}

private func MoveToGround() {
	var mov = 0, x = 0, y = 0;
	while (!GBackSolid(x, y) && mov < LandscapeWidth()) {
		RotateY(GetR(), ++mov, x, y);
	}
	RotateY(GetR(), mov - GetDefHeight() / 2, x, y);
	SetPosition(GetX() + x, GetY() + y);
}

private func MoveBehindTrees() {
	for (var obj in FindObjects(Find_AtPoint(), Find_OCF(OCF_Chop), Find_NoContainer(), Find_Category(C4D_StaticBack), Find_Func("IsTree")))
		SetObjectOrder(obj, this, 1);
}

private func Grow() {
	if(GetCon() > RandomX(60, 100))
		return;
	// Stop growing without terraformer.
	if(FindTerraformer(true))
		DoCon(Random(10));
	ScheduleCall(this, "Grow", 50);
}
