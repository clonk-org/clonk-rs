/*-- Chasma --*/

#strict

func Initialize() {
  MarsSky();
  SetSkyAdjust(RGBa(255,255,255,160), RGB(70,30,15));
  //Hintergrund
  CreateObject(BG_1,-0,0,-1);
  CreateObject(DUSK,0,0,-1);
  ScriptGo(1);

  // Spielziel setzen
  var goal = FindObject(RCOB);
  goal->AddType(DBSE);
  goal->AddType(DLAB);
  goal->AddType(DSOL);
  
  //Forschungsstation
  PlaceObjects(DBSE,1,"Durolava",950,1200,150,150);
  PlaceObjects(DLAB,1,"Durolava",950,1200,150,150);
  PlaceObjects(BONE,9,"Durolava",950,1200,150,150);
  PlaceObjects(SKUL,3,"Durolava",950,1200,150,150);    
  PlaceObjects(DSOL,3,"Durolava",950,1200,150,150);   
  PlaceObjects(METL,2,"Durolava",950,1200,150,150);   
  PlaceObjects(CELL,1,"Durolava",950,1200,150,150);   
  PlaceObjects(SATD,1,"Durolava",950,1200,150,150);
  PlaceObjects(ORE1,8,"Durolava",950,1200,150,150);  
  
  // Spouter
  PlaceObjects(OILS, RandomX(1,3), "Oil", 0, 1300, LandscapeWidth(), 300);  
  
  // Kristalle
  PlaceCrystals(5, 20);
  
  return(1);
}

protected func InitializePlayer(iPlr){
	var pCapsule = CreateObject(CPSL, RandomX(LandscapeWidth() / 2, LandscapeWidth() - 50), 70, iPlr);
	for(var pObj in FindObjects(Find_Owner(iPlr), Find_Or(Find_Category(C4D_Object), Find_OCF(OCF_CrewMember)))) {
		pObj -> Enter(pCapsule);
	}
	pCapsule->SetDstPort(0, true);
	// Startmaterial in der Kapsel
	CreateContents(CNKT, pCapsule, 2);
	CreateContents(METL, pCapsule, 2);
	CreateContents(SATD, pCapsule, 1);
	CreateContents(PSTC, pCapsule, 2);
	CreateContents(O2BL, pCapsule, 2)->DoFill(40);
}
  
//Regelwähler
func NotChoosableRules() { return [LNGT,ENRG,CNMT,RSNB]; }

// 1px-Wand ignorieren
func RightClosed() { return true; }
func LeftClosed() { return true; }
  
func Script8() {
  for(var i;i<1;i++) PlaceInMaterial(_BUB,Material("DuroLava"));
  goto(1);
  }

func RegulatedMaxTemp() {
	return 900;
}

func LowerCooling() { return 500; }

