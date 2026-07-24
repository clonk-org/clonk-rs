/*-- Spouter --*/

#strict 2

protected func Initalize() {
	Sound("Env_Spouter", 0, 0, 0, 0, +1);
}

protected func CastOil() {
  //Erdhack?
  DigFree(-1,-1,4);
	// steckt fest?
	if(GBackSolid()) {
		// langsam einen Weg nach oben finden!
		SetPosition(GetX() + RandomX(-1, 1), GetY() - 1);
		return;
	}
	// kein festes Material mehr unter uns?
	if(!GBackSolid(0, 2)) {
		// runterfallen!
		SetCategory(C4D_Vehicle);
		return;
	}
	
	for(var i = 0; i < 8; i++) {
		InsertMaterial(Material("Oil"), 0, 0, RandomX(-10, 10), RandomX(-40, -20));
	}
}

// wieder festen Grund gefunden
protected func Hit() {
	SetCategory(C4D_StaticBack);
}
