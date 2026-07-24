/*-- Eos --*/

#strict 2

func Initialize() {
  MarsSky();
  SetSkyAdjust(RGBa(200,94,51,90),RGB(200,110,55));
  //Hintergrund
  CreateObject(BG_2,-0,0,-1);
  PlaceCrystals(5, 20);
  return(1);
  }

protected func InitializePlayer(int iPlr) {
	var pCapsule = CreateObject(CPSL, RandomX(100, LandscapeWidth() - 100), 70, iPlr);
	for(var pObj in FindObjects(Find_Owner(iPlr), Find_Or(Find_Category(C4D_Object), Find_OCF(OCF_CrewMember)))) {
		pObj -> Enter(pCapsule);
	}
	pCapsule->SetDstPort(0, true);
}  
  
  //Regelwähler
func NotChoosableRules() { return [ENRG,CNMT]; }

func RegulatedMaxTemp() {
	return 425;
}
