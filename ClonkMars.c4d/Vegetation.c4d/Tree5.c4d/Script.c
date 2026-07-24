/*-- Baum --*/

#strict 2

#include TREE

protected func Construction() {
	if(!FindTerraformer()) { // Terraformingeinheit in der Nähe?
		RemoveObject();
		return;
	}
	AddEffect("TerraformerCheck", this, 10, 50, this);
	
	// Drehung nach Erdoberfläche
	var x_off = GetDefWidth(GetID()) / 2;
	var y_off = GetDefHeight(GetID()) / 2;
	var slope = GetSolidOffset(-x_off, y_off) - GetSolidOffset(x_off, y_off);
	SetR(slope*2); // passt besser
}

private func GetSolidOffset(int x, int y)
{
  var i;
  for (i = -20; GBackSolid(x, y - i) && (i < 20); i++);
  return i;
}

// Checks whether the tree stands in Incindiary material.
private func InLava() {
	return GetMaterialVal("Incindiary", "Material", GetMaterial(0, GetDefHeight(GetID()) / 2));
}

private func FindTerraformer(bool active) {
	var criteria = Find_And(Find_Distance(TRFM_RADIUS), Find_ID(TRFM));
	if (active) {
		criteria = Find_And(criteria, Find_Func("IsTerraforming"));
	}
	return FindObject2(criteria);
}

protected func FxTerraformerCheckTimer() {
	if(InLava() || !FindTerraformer(true)) { 
		DoCon(-1);
		if(GetCon() <= 1)
			RemoveObject();
		TreeHurt(5 + Random(10));
	}
}

private func TreeHurt(int iNum, int iLevel) {
	if(!iLevel)
		iLevel = 15 + Random(10);
	var ox, oy, wdt, hgt;
	GetRect(GetID(), ox, oy, wdt, hgt);
	ox = ox * GetCon() / 100;
	oy = oy * GetCon() / 100;
	wdt = wdt * GetCon() / 100;
	hgt = hgt * GetCon() / 100;
	iNum = iNum * GetCon() * 2 / 100;
	for(var cnt=0 ;cnt < iNum; cnt++)
		CreateParticle("GrassBlade",RandomX(ox, wdt), RandomX(oy, hgt),RandomX(-iLevel/3 ,iLevel/3),RandomX(-2*iLevel/3,-iLevel/3),30+Random(30),RGB(255,255,255),0,0);
}

protected func Damage() {
	TreeHurt(15 + Random(10));
	if (GetDamage() > TreeStrength() * Max(GetCon(), 30)/100) {
		TreeHurt(10);
		RemoveObject();
	}
}
