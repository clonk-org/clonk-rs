#strict 2

static iWindEffectedCount; //Used for optimisation of that array.
static aWindEffectedObjects;
static iGlobalWind; //Wind speed
static aWindForecast; //Two ints: Frame, speed to set at that frame
//static iGlobalHorizon;
static iLastGlobalHorizonCheck;
static iWindProfilingObjects;
static aBorderHeights;
static pWindControl;

//2do: Intelligent SRMP-Placing, proper SRMP-Counting
//Think about: 
//	Don't ignore the global horizon, go down in wind speed a bit when coming closer.
//	Check GetScenLeft/RightOpen() in Initialize
//	Rotate flying objects

global func AdvancedWindCalculations() {return ObjectCount(STRM);}

global func GetWind(int iX, int iY, bool fGlobal) {
	if(!pWindControl) return inherited(...);
	iX += GetX(); iY += GetY(); //Make stuff global
	if(fGlobal) return iGlobalWind;
	var iAng, iDst = (((iGlobalWind < 0) && LandscapeWidth()) - iX);
	//Note on && hack: if(iGlobalWind < 0) {second &&-condition is not to be checked and thus not returned} else {second condition is true, so it is returned}
	for(iAng=1;iAng<45; iAng+=15) { //Itering in 15°, subtest once with -8; If you change, take care that max is at 45°
		if(PathFree(iX, iY, (iGlobalWind < 0) && LandscapeWidth(), Sin(iAng,iDst)*iDst/Cos(iAng,iDst))) {
			if(iAng > 1 && PathFree(iX, iY, (iGlobalWind < 0) && LandscapeWidth(), Sin(iAng-8,iDst)*iDst/Cos(iAng-8,iDst)))
				iAng -= 8;
			break;
		}
	}
	return iGlobalWind * (45-iAng) / 45 * (Random(51) + 975) / 1000; //* (((GetHorizon()>iY) && (GetHorizon()-iY)) || 200) / 200
}

global func SetWind(int iWind, int iHold) {
	if(!pWindControl) return inherited(...);
	iHold += FrameCounter();
	iWind = BoundBy(iWind,-250,250);
	var i;
	/*while(i<GetLength() && aWindForecast[i][0] < iHold) ++i;
	PopElements(aWindForecast, 1, i-1);*/
	aWindForecast = [[FrameCounter(), iWind], [iHold, iWind]];
}

global func IsStormy() {
	return Abs(iGlobalWind) > 200;
}

protected func Initialize() {
	if(pWindControl) return RemoveObject();
	pWindControl = this;
	aWindForecast = [[0, Random(101)-50]]; //Moderate at the beginning
	aWindEffectedObjects = [];
	aBorderHeights = [];
	if(!GetEffect("IntWindExec")) AddEffect("IntWindExec", 0, 1, 3, 0, STRM);
	var i, posy;
	for(var posx in [0,LandscapeWidth()]){
		posy=0;
		while(!GBackSolid(posx,posy)) posy++;
		aBorderHeights[i] = posy;
		i++;
	}
}

global func FxIntWindExecTimer() {
	// Update Forecast
	if(aWindForecast[GetLength(aWindForecast)-1][0] - FrameCounter() < 10000)
		var frm = aWindForecast[GetLength(aWindForecast)-1][0]+Random(101)+Random(101); //Hack to avoid funny engine 'bug'
		var dir = BoundBy(aWindForecast[GetLength(aWindForecast)-1][1]+Random(121)-60, -250, 250); //If you put it directly into the array, it fails. That's because the array's extended, before the expression is evaluated.
		aWindForecast[GetLength(aWindForecast)] = [frm,dir]; 
	// Calc speed
	if(aWindForecast[0][0] < FrameCounter()) {
		if(aWindForecast[0][1] != iGlobalWind) {
			if(iGlobalWind < aWindForecast[0][1]) iGlobalWind += 1;
			else iGlobalWind -= 1;
			if(Abs(aWindForecast[0][1] - iGlobalWind) < 3) iGlobalWind = aWindForecast[0][1]; //Correctness jump, I don't want it to hang
		} else {
			PopElements(aWindForecast, 0);
		}
	}
	// Effect Objects
	var i = iWindEffectedCount;
	while(i--) {
		if(!aWindEffectedObjects[i]) {
			aWindEffectedObjects[i] = aWindEffectedObjects[--iWindEffectedCount]; 
			if(GetLength(aWindEffectedObjects) - iWindEffectedCount > 20) SetLength(aWindEffectedObjects,iWindEffectedCount+12);
			continue;
		}
		var iDir = aWindEffectedObjects[i]->GetWind(), iDiff = iDir*10-GetXDir(aWindEffectedObjects[i],100);
		SetXDir((iDiff>0)-(iDiff<0) /*vorzeichen*/ * (iDiff)**2 * aWindEffectedObjects[i]->WindEffect() / 100000 +
			GetXDir(aWindEffectedObjects[i],100),aWindEffectedObjects[i],100);
			//Formula Dependencies: Capsule::SetDstPort()
	}
	// Effect Landscape
	while(iWindProfilingObjects < (Max(Abs(iGlobalWind)-30, 0)**3 / 12500)) {
		RePlaceWindProfiler(CreateObject(SRMP, 0, 0, NO_OWNER));
		++iWindProfilingObjects;
	}
	if(!(FrameCounter()%30)) iWindProfilingObjects = ObjectCount(SRMP); //Something is eating them. 2do FIXME
}

global func RePlaceWindProfiler(object pProf) {
	if(iWindProfilingObjects > (Max(Abs(iGlobalWind)-30, 0)**3 / 25)) {
		--iWindProfilingObjects;
		RemoveObject(pProf);
		return;
	}
	SetPosition(Random(LandscapeWidth()), 1, pProf);
	pProf->SetYDir(100);
}

global func AddObjectForWind(object pObj) {
	if(!pWindControl) return;
	if(!pObj) if(!(pObj=this)) return;
	if(GetLength(aWindEffectedObjects) >= iWindEffectedCount) SetLength(aWindEffectedObjects,iWindEffectedCount+15);
	aWindEffectedObjects[iWindEffectedCount++] = pObj;
}

global func RemoveObjectFromWind(object pObj) {
	if(!pWindControl) return;
	if(!pObj) if(!(pObj=this)) return;
	var pos;
	if(pos=GetIndexOf(pObj, aWindEffectedObjects)) {
		aWindEffectedObjects[pos] = aWindEffectedObjects[--iWindEffectedCount];
		if(GetLength(aWindEffectedObjects) - iWindEffectedCount > 20) SetLength(aWindEffectedObjects,iWindEffectedCount+12);
		return true;
	}
	return false;
}

/*global func GetHorizon() {
	if(FrameCounter() - iLastGlobalHorizonCheck < 30) return iGlobalHorizon; //Assuming it won't be that obvious if the effect is delayed for a second.
	while(PathFree(0, iGlobalHorizon, GetLandscapeWidth(), iGlobalHorizon)) iGlobalHorizon += 5;
	while(!PathFree(0, iGlobalHorizon, GetLandscapeWidth(), iGlobalHorizon)) iGlobalHorizon -= 5;
	iLastGlobalHorizonCheck = FrameCounter();
	return iGlobalHorizon;
}*/