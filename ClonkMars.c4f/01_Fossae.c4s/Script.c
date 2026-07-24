/*-- Fossae --*/

#strict 2

func Initialize() {
  MarsSky();
  SetSkyParallax(0,10,0,3,0);
  DigBuildingsFree();
  ScriptGo(1);
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

protected func Script30(){
  //place Dust
  for(var i;i<10;++i) 
  CreateParticle("Dust",Random(LandscapeWidth()),Random(LandscapeHeight()*2/3),RandomX(3,9),0,RandomX(1000,1500));
  return(goto(0));
  }
  
//Regelwähler
func NotChoosableRules() { return [LNGT,LDAY,ENRG,CNMT]; }

func LowerCooling() { return 75; }

func RegulatedMaxTemp() {
	return 300;
}
