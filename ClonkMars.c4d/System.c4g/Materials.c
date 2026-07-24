/* Material colors */

#strict 2

#appendto ROCK
#appendto ERTH

static aMaterialObjectColors;

protected func Initialize() {
	if     (GetID() == ROCK) SetClrModulation(GetMaterialObjectColor(Material("Rock")));
	else if(GetID() == ERTH) SetClrModulation(GetMaterialObjectColor(Material("Earth")));
	return _inherited(...);
}

global func GetMaterialObjectColor(int iMat) { //Overload in Scenario if you like
	if(GetType(aMaterialObjectColors) != C4V_Array) aMaterialObjectColors = CreateArray();
	if(aMaterialObjectColors[iMat]) return aMaterialObjectColors[iMat]-1;
	var clr = CreateArray(3);
	for(var i=0; i<3; i++) {
		for(var j=0; j<3; j++) {
			clr[i] += GetMaterialColor(iMat,j,i);
		}
		clr[i] /= 3;
	}
	var max = 0;
	for(var i=1; i<3; i++) if(clr[i] > clr[max]) max = i;
	if(clr[max] < 20) { 
		aMaterialObjectColors[iMat] = RGB(255,255,255) + 1;
		return RGB(255,255,255);
	}
	var maxv = clr[max];
	for(var i=0; i<3; i++) clr[i] = ((0x00ff00 - (maxv - clr[i])*100) / maxv * clr[i]) >> 8;
	clr[max]=255;
	aMaterialObjectColors[iMat] = RGB(clr[0],clr[1],clr[2]) + 1;
	return RGB(clr[0],clr[1],clr[2]);
}