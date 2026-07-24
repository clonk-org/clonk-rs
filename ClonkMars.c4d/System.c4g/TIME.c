/* Temperaturanpassung abhängig von der Tageszeit */

#strict 2

#appendto TIME

global func TimeResolution() { return 10000; }

global func GetTimePoints(&evening, &morning) {
	evening = TimeResolution() / 4;
	morning = TimeResolution() * 3 / 4;
	var pObj = FindObject2(Find_Category(C4D_Rule), Find_Func("OverloadsTime"));
	if(pObj)
		pObj -> TimeOverload(evening, morning);
	return 1;
}

private func GetLightIntensity(int iTime) {
	var evening, morning;
	GetTimePoints(evening, morning);
	
	var fade = TimeResolution() / 100;
	evening -= fade / 2;
	morning -= fade / 2;
	
	// ganz hell?
	if(iTime < evening || iTime > morning + fade)
		return 100;
	
	if(iTime - evening > fade)
		iTime -= morning;
	else
		iTime -= evening;
	
	return Max(0, iTime * 100 / fade);
} 

private func CalcTemperature(int iTime) {
	if(FindObject2(Find_ID(RSNB))) // Regel "Ewige Kälte" aktiviert?
		return GetLightIntensity(GetTime()) * (-20 - -80) / 100 + -80; // Temperatur nicht über -20 steigen lassen
	return GetLightIntensity(GetTime()) * (80 - -80) / 100 + -80;
}

public func SetTime() {
	inherited(...);
	SetTemperature(CalcTemperature(Local(1)));
	//UpdateHUD(NO_OWNER, HUD_Temp, Round(GetTemperature()));
}

private func Timing() {
	inherited(...);
	SetTemperature(CalcTemperature(Local(1)));
	//UpdateHUD(NO_OWNER, HUD_Temp, Round(GetTemperature()));
}

private func Round(int iValue) {
	var iMod = iValue % 20;
	if(iMod < 10)
		iValue -= iMod;
	else
		iValue += 20 - iMod;
	return iValue;
}

protected func Initialize() {
	if(!ObjectCount2(Find_ID(DARK)))
		CreateObject(DARK, 0, 0, NO_OWNER);
	SetDarkness(GameCall("MinDarkness"));
	return _inherited(...);
}

private func RestoreSkyColors(int iPercent) {
	iPercent = 100 - iPercent;
	var iMin = GameCall("MinDarkness"), iMax = GameCall("MaxDarkness");
	if(!iMax)
		iMax = 62; // <Nachtfalter> einigen wir uns auf 62
	iPercent = iPercent * (iMax - iMin) / 100 + iMin;
	FadeDarkness(iPercent);
	return 1;
}
