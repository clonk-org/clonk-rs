/*-- Snowman --*/

#strict 2

protected func Initialize() {
	if(!FindObject2(Find_ID(TIME))) {
		SetTemperature(-20);
		UpdateHUD(NO_OWNER, HUD_Temp, -20);
	}
}

protected func Activate(iPlr) {
  MessageWindow(GetDesc(),iPlr);
}

