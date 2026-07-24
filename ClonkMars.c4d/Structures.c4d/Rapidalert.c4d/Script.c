/*-- RAS --*/

#strict 2
#include B_40
#include DACT //Damagecontrol
//#include PWRC // Energieverbraucher

protected func Initialize(){
  SetAction("Active",this());
  }

protected func WarningMeto(){
  Sound("Warning_meto");
  }

public func MaxDamage() { return 32; } //Maximaler Schaden
public func MarsResearch() {return true;}
