/* Viewport */

#strict 2

local activated;
local wdt, hgt;
local freeze, blood;

// viewport padding in px
static const VWPT_Padding = 80;

// overlay layer
static const VWPT_Blood = 1;
static const VWPT_Freeze = 2;

protected func Initialize() {
	SetPosition(0, 0);
	SetVisibility(VIS_Owner);
	SetAction("Invisible");
	
	freeze = 255;
	blood = 255;
	UpdateViewportSize();
}

private func GetRes(int index) {
	var res = [
		// 4:3
		[800, 600],
		[1024, 768],
		[1280, 1024],
		
		// 16:9
		[1280, 720],
		[1920, 1080],
		
		// Sonstige
		[1366, 768],
		[1680, 1050]
	];
	
	if(index < 0) {
		return res;
	}
	
	return res[index];
}

public func SizeSelection() {
	var menu = CreateMenuTemplate(GetID(), "$MenuTitle$");
	AddEnumChoice(menu, 0, "res", 0, "$MenuResolution$");
	
	// Vordefinierte Auflösungen
	var res = GetRes(-1);
	for(var i = 0; i < GetLength(res); i++) {
		AddEnumChoiceItem(menu, 0, "res", i, Format("%dx%d", res[i][0], res[i][1]), 0, res[i]);
	}
	
	// Benutzerdefiniert
	AddEnumChoiceItem(menu, 0, "res", "user", "$MenuCustom$", 0, 0, true);
	AddRangeChoice(menu, 0, "X", MenuCond_Chosen(0, "res", "user"), "$MenuX$", 0, 1, 3000, 1, wdt);
	AddRangeChoice(menu, 0, "Y", MenuCond_Chosen(0, "res", "user"), "$MenuY$", 0, 1, 3000, 1, hgt);
	
	CreateMenuByTemplate(GetCursor(GetOwner()), this, "ChooseSize", menu);
	
	return 1;
}

public func ChooseSize(array result, ExtraData, bool canceled) {
	// bei Schließen durch Graben nichts tun
	if(canceled)
		return;
	
	var res = HashGet(result, "res");
	if(!res) {
		wdt = HashGet(result, "X");
		hgt = HashGet(result, "Y");
	}
	else {
		wdt = res[0];
		hgt = res[1];
	}
	
	// abspeichern!
	var iPlr = GetOwner();
	SetPlrExtraData(iPlr, "ScreenWidth", wdt);
	SetPlrExtraData(iPlr, "ScreenHeight", hgt);
	
	UpdateViewportSize();
	
	/*var iter = HashIter(result), node;
	while(node = HashIterNext(iter)) {
		Log("%v - %v", node[0], node[1]);
	}*/
}

public func UpdateViewportSize() {
	var iPlr = GetOwner();
	wdt = GetPlrExtraData(iPlr, "ScreenWidth");
	hgt = GetPlrExtraData(iPlr, "ScreenHeight");
	if(!wdt) {
		wdt = 1024;
		hgt = 768;
		activated = false;
	}
	else {
		activated = true;
		SetFreeze(freeze);
	}
	// Anpassung bei kleiner Landschaft
	wdt = Min(wdt, LandscapeWidth() + VWPT_Padding);
	hgt = Min(hgt, LandscapeHeight() + VWPT_Padding);
}

global func SetViewportFreeze(int iPlr, int iFreeze) {
	var pObj = FindObject2(Find_ID(VWPT), Find_Owner(iPlr));
	if(pObj)
		return pObj -> SetFreeze(iFreeze);
}

public func SetFreeze(int iFreeze) {
	if(activated) {
		// bisher nichts?
		if(iFreeze != 255 && freeze == 255) {
			SetGraphics("Frosty", this, GetID(), VWPT_Freeze, GFXOV_MODE_Base);
			SetObjDrawTransform(1000 * wdt / 1024, 0, 0, 0, 1000 * hgt / 768, 0, this, VWPT_Freeze);
		}
		
		if(iFreeze == 255) {
			if(freeze != 255) {
				SetGraphics(0, this, 0, VWPT_Freeze);
			}
		}
		else
			SetClrModulation(RGBa(255, 255, 255, iFreeze), this, VWPT_Freeze);
	}
	
	freeze = iFreeze;
	
	return 1;
}

global func DoViewportBlood(int iPlr) {
	var pObj = FindObject2(Find_ID(VWPT), Find_Owner(iPlr));
	if(pObj)
		return pObj -> DoBlood();
}

public func DoBlood() {
	if(!activated)
		return;
	SetBlood(0);
	AddEffect("BloodFade", this, 10, 1, this);
	return 1;
}

protected func FxBloodFadeTimer() {
	SetBlood(Min(blood + 2, 255));
	if(blood == 255)
		return -1;
}

protected func FxBloodFadeEffect(string szNewEffectName) {
	if(szNewEffectName == "BloodFade")
		return -2;
}

public func SetBlood(int iBlood) {
	if(!activated)
		return;
	
	// bisher nichts?
	if(iBlood != 255 && blood == 255) {
		SetGraphics("Bloody", this, GetID(), VWPT_Blood, GFXOV_MODE_Base);
		SetObjDrawTransform(1000 * wdt / 1024, 0, 0, 0, 1000 * hgt / 768, 0, this, VWPT_Blood);
	}
	
	if(iBlood == 255) {
		if(blood != 255) {
			SetGraphics(0, this, 0, VWPT_Blood);
		}
	}
	else
		SetClrModulation(RGBa(255, 255, 255, iBlood), this, VWPT_Blood);
	
	blood = iBlood;
	
	return 1;
}
