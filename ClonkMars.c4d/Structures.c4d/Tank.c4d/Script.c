/*-- Tank --*/

#strict 2
#include B_60
#include DACT //Damagecontrol
#include L_SS
#include L_CA //Baugrafik

local type; // 0: Öl; 1: Lava

func Initialize() {
	UpdatePicture();
	return 1;
}

public func TankFor(int mat) {
	if(type == 0)
		return mat == Material("Oil");
	else
		return WildcardMatch(MaterialName(mat), "*Lava*");
}

public func MaxFill() {
	if(type == 0)
		return 3500;
	else
		return 8750;
}

private func FillPicture() { return; }

private func UpdatePicture() {
	if(GetCon() != 100)
		return;

	// Der Bohrturm füllt evtl. nicht ganz
	var iNum = GetFill() * 5 / MaxFill();
	if(MaxFill() - GetFill() < 5)
		iNum = 5; // trotzdem als voll anzeigen

	var gfx = 0;
	if(ContainsLava())
		gfx = "Lava";
	SetGraphics(gfx, this, _BAR, 1, GFXOV_MODE_Action, Format("%d/5", iNum));
	SetObjDrawTransform(1000, 0, -8000, 0, 1000, 3500, this, 1);

	UpdateTypePicture();
	return 1;
}

private func UpdateTypePicture() {
	var act = ContentType();
	if(IsFull())
		act = Format("%sFull", act);
	SetGraphics(0, this, PLAT, 2, GFXOV_MODE_Action, act);
	SetObjDrawTransform(1000, 0, 33000, 0, 1000, -2000, this, 2);
	return 1;
}

public func Deconstruction() {
	//Overlay
	SetGraphics(0, this, 0, 1);
	
	return _inherited(...);
}

public func PipelineConnect() {
	return 1;
}

//Steuerung
  
protected func ControlUp(){
  if(!Hostile(GetOwner(),GetOwner(Par()))){
  Message("%d",this,GetFill());}
    else{
      Message("$TxtEnemy$",this());
      Sound("Error");}
  }

protected func ControlDig(object pClonk) {
	if(Hostile(GetOwner(), pClonk->GetOwner())) {
		Message("$TxtEnemy$");
		Sound("Error");
		return;
	}
	if(type) {
		Message("$TxtOnlyOil$", pClonk);
		Sound("Error");
		return;
	}

	var iChange = pClonk -> ~DoFuel(GetFill() * 100);
	iChange = ChangeRange(iChange, 0, pClonk -> ~MaxFuel(), 0, 100);
	Message("+%d", pClonk, iChange);
	pClonk -> ~UpdateFuelHUD();
	DoFill(-iChange);
}

protected func ControlThrow(object clonk) {
	if(!Hostile(GetOwner(), clonk->GetOwner())) {
		if(GetFill())
			return Message("$TxtOnlyEmptyTank$", this);
		type = !type;
		UpdateTypePicture();
	} else {
		Message("$TxtEnemy$",this());
		Sound("Error");
	}
	return 1;
}

protected func ContextSwitch2Oil() {
	[$TxtSwitchToOil$|Image=OILT|Condition=ContainsLava]
	ControlThrow(...);
}

protected func ContextSwitch2Lava() {
	[$TxtSwitchToLava$|Image=OILT|Condition=ContainsOil]
	ControlThrow(...);
}

public func ContainsOil() {
	return type == 0;
}

public func ContainsLava() {
	return type == 1;
}

public func ContentType() {
	if(ContainsOil())
		return "Oil";
	if(ContainsLava())
		return "Lava";
}

// Returns the name of the material to be evicted.
public func ContentMaterial() {
	if(ContainsOil())
		return "Oil";
	if(ContainsLava())
		return "DuroLava";
}

protected func NotEmpty() {
	return GetFill() != 0;
}

protected func ContextFlush() {
	[$TxtFlushTank$|Image=OILT|Condition=NotEmpty]
	var mat = ContentMaterial();
	CastPXS(mat, Abs(DoFill(-MaxFill())), 20);
	return 1;
}

/* Erforschbar */

public func MarsResearch() {
	return true;
}

public func MaxDamage() { return 20; } //Maximaler Schaden

private func DestroyBlast(object pTarget) {
	if(!pTarget) if(!(pTarget=this)) return;
	var iAmount = pTarget -> GetFill();
	var mat = ContentMaterial();
	pTarget -> CastPXS(mat, RandomX(iAmount / 2, iAmount), 50);
	return inherited(...);
}

public func Deconstruction() {
	var mat = ContentMaterial();
	CastPXS(mat, Abs(DoFill(-MaxFill())), 20);
	return _inherited(...);
}

/* Anzeige-fix nach Spielstand laden */

func UpdateTransferZone()
{
	UpdatePicture();
}
