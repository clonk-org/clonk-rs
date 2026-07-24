/* Lorenteleport */

#strict 2

#appendto LORY

protected func ControlUp(object pClonk) {
	var pStation = FindObject2(Find_AtPoint(), Find_ID(TRNS));
	if(pStation) {
		pStation -> ControlUp(pClonk, false, this);
		return 1;
	}
}

protected func ControlDig(object pClonk) {
	var pStation = FindObject2(Find_AtPoint(), Find_ID(TRNS));
	if(pStation) {
		pStation -> ControlUp(pClonk, true, this);
		return 1;
	}
}
