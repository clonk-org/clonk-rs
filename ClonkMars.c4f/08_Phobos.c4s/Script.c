/*-- Phobos --*/

#strict 2


func Initialize() {
	//rötlich wegen GI
	SetGamma(RGB(30,0,0), RGB(120,100,100), RGB(235,220,220));
	DigBuildingsFree();
	// Spielziel setzen
	FindObject(CROB)->AddType(MRAS, 2);
	FindObject(CROB)->AddType(RADO, 4);
	FindObject(CROB)->AddType(PORT, 1);

	CreateObject(MARS,-0,0,-1);

	AddEffect("FragmentRain", 0, 10, 250);
}

protected func InitializePlayer(int iPlr) {
	var pCapsule = CreateObject(CPSL, RandomX(100, LandscapeWidth() - 100), 70, iPlr);
	for(var pObj in FindObjects(Find_Owner(iPlr), Find_Or(Find_Category(C4D_Object), Find_OCF(OCF_CrewMember)))) {
		pObj -> Enter(pCapsule);
	}
	pCapsule->SetDstPort(0, true);
}

global func FxFragmentRainTimer(object target, int effectNum, int effectTime) {
	if(!Random(10)) {
		var fragment = CreateObject(ORE1, Random(LandscapeWidth()), -10, NO_OWNER);
		fragment->SetXDir(RandomX(-200, 200));
		fragment->SetYDir(RandomX(70, 150));
		AddEffect("MeteorTrail", fragment, 100, 1, fragment);
	}
}

global func FxMeteorTrailTimer(object target, int effectNum, int effectTime) {
	// Until we stop.
	if (GetYDir() <= 0) return -1;

	Smoke(0, 0, 5);
	Smoke(0, -5, Random(7));
	var i = Random(50);
	while (i--) {
		var c = 20+Random(41);
		CreateParticle("PxSpark", Random(11)-5, Random(11)-5, 0, 0, c, RGBa(255, Random(100), c*4, 128));
	}
	CreateParticle("PxSpark", 0, 0, 0, 0, 20, RGBa(255, 155, 0, 128));
}

//Regelwähler
func NotChoosableRules() { return [ENRG,CNMT]; }

func LowerCooling() { return 40; }

func RegulatedMaxTemp() {
	return 280;
}
