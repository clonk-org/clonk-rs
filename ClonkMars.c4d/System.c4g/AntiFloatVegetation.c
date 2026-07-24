/* Stop some kind of vegetation from growing in the air. */

#strict 2

#appendto GRAS //Grass
#appendto TRE5 //Tree
#appendto TRE6 //Tree
#appendto TRE7 //Wompfberrytree
#appendto CRY2 //Crystals

protected func Construction() {
	AddEffect("CheckAir", this, 1, 30, this);
	return _inherited(...);
}

protected func FxCheckAirTimer(object target, int effectNum, int effectTime) {
	var x, y;
	RotateY(GetR(), GetDefHeight() / 2 + 2, x, y);
	var mat = GetMaterial(x, y);

	if(GetID() == CRY2) {
		if(GBackSolid(x, y))
			return;
	} else {
		if(GetMaterialVal("Soil", "Material", mat))
			return;
	}

	// None of the criteria above matched - die fast.
	DoCon(-50);
}
