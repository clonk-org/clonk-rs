/*-- Abyss --*/

#strict 2

func Initialize() {
  //typical Mars Sky
  SetSkyAdjust(RGBa(190,143,65,255),RGB(195,143,59));
  SetGamma(RGB(50,0,0), RGB(140,100,100), RGB(255,220,220));
  SetSkyParallax(0,10,0,3,0);
  var Rain=CreateObject(FXP1,0,0,-1);
  Rain->Activate(Material("DuroLava"),960,50);
  ScriptGo(1);
  var rSmoke=CreateObject(BG_4, LandscapeWidth()/2, 350); //Render-Rauch
    rSmoke -> SetClrModulation(RGBa(200,190,128,150));
    
  var vSmoke4=CreateObject(_V01,700,300);
    vSmoke4 -> SetClrModulation(RGBa(200,190,128,10));
    
  var vSmoke5=CreateObject(_V01,705,310);
   vSmoke5 -> SetClrModulation(RGBa(200,190,128,10));
   
  var vSmoke6=CreateObject(_V01,695,310);
  vSmoke6 -> SetClrModulation(RGBa(200,190,128,10));
    
  var vSmoke2=CreateObject(_V01,690,300);
    vSmoke2 -> SetClrModulation(RGBa(200,190,128,10));
    
  var vSmoke3=CreateObject(_V01,710,300);
    vSmoke3 -> SetClrModulation(RGBa(200,190,128,10));
    
  CreateObject(_V03,700,300);
  
  var Vulcano=CreateObject(BG_3, LandscapeWidth()/2, 560);
    Vulcano -> SetClrModulation(RGBa(200,190,128,12));
  CreateObject(S_VO,0,0,-1); //Sound dafür
  SetWind(Random(42));
  
  CreateObject(OILS,465,540, -1);
  
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

protected func Script2(){
  //place Dust
  for(var i;i<15;++i) 
  CreateParticle("Dust",Random(LandscapeWidth()),Random(LandscapeHeight()*2/3),RandomX(3,9),0,RandomX(1000,1500));
  return(goto(0));
  }
  
//Regelwähler
func NotChoosableRules() { return [LNGT,LDAY,ENRG,CNMT]; }

func LowerCooling() { return 20; }

func RegulatedMaxTemp() {
	return 900;
}

//Spielziel
func TerraformingGoal(){ return(50);}
