/* Schützt vor Erde vor Gebäuden */

#strict 2

global func DigBuildingsFree() {
	return AddEffect("DigBuildingsFree", 0, 100, 100);
}

global func FxDigBuildingsFreeTimer() {
	var ID;
	for(var pObj in FindObjects(Find_Category(C4D_Structure), Find_OCF(OCF_Fullcon), Find_Not(Find_Func("NoDigFree")))) {
		ID = pObj -> GetID();
		DigFreeRect(pObj -> GetX() + GetDefCoreVal("Offset", "DefCore", ID, 0), 
		            pObj -> GetY() + GetDefCoreVal("Offset", "DefCore", ID, 1), 
		            GetDefWidth(ID), 
		            GetDefHeight(ID));
	}
}
