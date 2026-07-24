/* ConstructAction */

#strict 2

public func ConstructActionLength() {
	var i = 0;
	while (GetActMapVal("Length", Format("Construct%d", i++))); 
	return i;
}

public func ConstructOverlay() { return 3; }

public func Construction () {
	AddEffect("ConstructGraphic", this, 1, 1, 0, L_CA);
	return _inherited(...);
}

public func FxConstructGraphicTimer (object pTarget, int iEffectNumber, int iEffectTime) {
	if (!pTarget) return -1;
	if (GetCon(pTarget) >= 100) {
		if(!EffectVar(0, pTarget, iEffectNumber)) {
			SetGraphics(0, pTarget);
			SetGraphics(0, pTarget, 0, ConstructOverlay());
			
			EffectVar(0, pTarget, iEffectNumber) = true;
		}
	}
	else {
		var state = GetCon(pTarget) * pTarget->ConstructActionLength() / 100 - 1;
		SetGraphics(0, pTarget, L_CA, 0, GFXOV_MODE_ExtraGraphics); //Grundlayer unsichtbar
		SetGraphics(0, pTarget, GetID(pTarget), ConstructOverlay(), GFXOV_MODE_Action, Format("Construct%d", state));
		var offy = GetDefCoreVal("Offset", "DefCore", GetID(pTarget), 1);
		var YAdjust = (1000 - GetCon(pTarget) * 10) * offy;
		SetObjDrawTransform(1000, 0, 0, 0, 1000, YAdjust, pTarget, ConstructOverlay());
		
		EffectVar(0, pTarget, iEffectNumber) = false;
	}
}
