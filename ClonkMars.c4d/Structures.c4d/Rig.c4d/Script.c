/*-- Rig --*/

#strict 2
#include DRCK //Original Clonk-Derrick. Be carefull with callbacks...
#include B_50
#include PWRC
#include DACT //Damagecontrol
#include L_CA //Baugrafik


local pLamp;
func Initialize() {
  SetAction("Wait");
  pLamp = AddLightCone(1000, RGBa(255, 255, 255, 10));
	pLamp -> ChangeSizeXY(800, 1000);
	pLamp -> ChangeOffset(-5,-37);
	pLamp -> ChangeR(105);
	pLamp -> LocalN("bAmbience") = true;
  return _inherited(...);
}

protected func Incineration() {
	pLamp -> RemoveObject();
	pDrillHead -> RemoveObject();
	return _inherited(...);
}

public func Deconstruction() {
	pLamp -> RemoveObject();
	pDrillHead -> RemoveObject();
	return _inherited(...);
}

protected func Pumping() {
	if(!ReadyToPump() || GetEffect("EnergyNeed", this))
		SetAction("Wait", pDrillHead);
}

private func PipeHeadCheck()
{
  // Bei Bedarf Bohrkopf und Leitung erzeugen
  if (!pDrillHead) 
  {              
    // Bohrkopf
    pDrillHead = CreateObject(PIPH, 13, 36, GetOwner());
    SetAction("Pump", pDrillHead);
    pDrillHead -> SetAction("Stop", this);
    CreateObject(DPLI, 13, 0, -1)->DPLI::Init(this, pDrillHead);
  }
  else {
  	if(GetAction() != "Pump")
  		SetAction("Pump", pDrillHead);
  }
  return(pDrillHead);
}

public func Transfer(int iAmount, int iMaterial, bool fNoChange) {
	for(var pTank in FindObjects(Find_Category(C4D_Structure), Find_Func("TankFor", iMaterial), Find_Func("PipelineConnectedWith", this))) {
		if(pTank -> GetFill() + iAmount <= pTank -> MaxFill()) {
			if(!fNoChange) {
				pTank -> DoFill(iAmount);
				if(MaterialName(iMaterial) == "Oil")
					Overburn();
			}
			return 1;
		}
	}
	return;
}

/*Fackel*/

protected func Overburn(){
  CreateParticle("Fire2", -10, -35, RandomX(-5,5), -10-Random(10), 75 + Random(75), RGBa(255, 230, 230, 50), this());
  CreateParticle("Fire2", -10, -30, RandomX(-5,5), -10-Random(10), 75 + Random(75), RGBa(255, 230, 230, 50), this());
  if(!Random(3))
    CreateParticle("Fire", -10, -30, RandomX(-5,5), -10-Random(10), 55, RGBa(255, 230, 230, 50), this());
    }

public func MaxDamage() { return 25; } //Maximaler Schaden

public func PipelineConnect() {
	return 1;
}

public func MarsResearch() {
	return true;
}

public func GetResearchBase() { return; }

