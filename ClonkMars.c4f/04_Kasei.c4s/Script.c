/*-- Kasei Vallis --*/

#strict

func Initialize() {
  MarsSky();
  SetSkyAdjust(RGBa(255,255,255,160), RGB(70,30,15));
  PlaceCrystals(5, 20);
  return(1);
}

protected func InitializePlayer(iPlr){
  //ersten Clonk alles in die Hand geben
  CreateContents(CNKT,GetCrew(iPlr,0),2);
  CreateContents(METL,GetCrew(iPlr,0),2);
  CreateContents(SATD,GetCrew(iPlr,0),1);
  CreateContents(PSTC,GetCrew(iPlr,0),2);
  return(1);
  }

  //Regelwähler
  func NotChoosableRules() { return [LNGT,ENRG,CNMT]; }

func RegulatedMaxTemp() {
	return 950;
}
