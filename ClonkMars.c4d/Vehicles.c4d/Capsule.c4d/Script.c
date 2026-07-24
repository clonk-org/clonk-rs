/*--Kapsel--*/

#strict 2

#include DACT
#include SRMO

public func GetTemp() { return 250; } // klimatisiert

local port, portvertex, origx;
local mode, horrblow, vertblow; //mode: 1 for automatic; 
local sat;

static const iCapsMaxSpeed  = 2500; //iPrecision = 500
static const iCapsLandSpeed = 250;
static const iCapsAcceleration = 30;

public func SetHorrBlowout(int bo) {
	if(horrblow != bo) {
		if(bo && !horrblow) DoCpslSound(1);
		else if(!bo) DoCpslSound(-1);
		horrblow = bo;
		if(bo == -1) SetAction("LeftBoostTurnUp");
		else if(bo == 1) SetAction("RightBoostTurnUp");
		else if(WildcardMatch(GetAction(), "RightBoost*")) SetAction("RightBoostTurnDown");
		else SetAction("LeftBoostTurnDown");
	}
	if(bo && !GetEffect("Blowout", this)) {AddEffect("Blowout", this,1,1,this);}
}

public func SetVertBlowout(int bo) {
	if(vertblow != bo) {
		DoCpslSound(bo-vertblow);
		vertblow = bo;
		if(bo && !GetEffect("Blowout", this)) {AddEffect("Blowout", this,1,1,this);}
	}
}

protected func Initialize() {
	portvertex = -1;
	SetAction("FreeFall");
	SetComDir(COMD_Down);
	SetYDir();
	origx = GetX();
	return _inherited();
}

public func SetDstPort(object pPort, bool noauto) {
	if(GetGravity() < 1) { Log("Can't land without gravity."); Explode(3000); return; }
	port = pPort;
	var dst;
	if(pPort) {
		dst = Abs(GetY()-GetY(pPort));
		pPort->Occupy(this);
		origx = GetX(pPort);
	} else {
		dst = Min(Abs(GetY()-GetHorizon(-24)) - 150, Abs(GetY()-GetHorizon(24)) - 150);
		origx += RandomX(-400,400);
	}
	if(dst < 1) dst = 1;
	var iter = dst / GetGravity(), t;
	if(iter < 1) iter = 1;
	var capsacc = iCapsAcceleration/20, gravacc = GetGravity()/20, landspd = iCapsLandSpeed/20;
	while(true){
		t+= iter;
		var i=t,d=0,s=0;
		while(i--) {
			s += gravacc;
			d += s;
		}
		while(s > landspd) {
			s -= capsacc;
			d += s;
		}
		if(d/20 > dst) {
			if(iter == 1) break;
			else {
				t -= 2*iter;
				iter /= 2;
			}
		}
	}
	if(AdvancedWindCalculations()) {
		var dir,pos=0;
		for(var i;i<t;++i) { //FIXME: Use forecast. And fix that whole thingy
			dir += (dir-GetWind(0,0,true))**2*WindEffect()/1000;
			pos += dir;
		}
		if(GetWind(0,0,true)<0) pos *= -1;
		SetPosition(GetX()-pos/100, GetY());
		AddEffect("Blowout", this,1,1,this);
	}
	ScheduleCall(this, "StartLanding", t);
	if(port)
		ScheduleCall(port, "PortActive", 50);
	if(!noauto) mode = 1;
	return 1;
}

public func SetSat(pSat) {
	sat = pSat;
}

private func DestroyBlast() {
	if(sat) sat -> CapsuleDestroyed();
	return _inherited(...);
}

global func GetHorizon(int iX) { var iY; while(!GBackSemiSolid(iX,AbsY(iY++)) && iY < LandscapeHeight()); return iY; }

// hat einen Sauerstoffspeicher zum Landen
public func IsO2Producer() {
	return 1;
}

protected func Hit(int iXDir, int iYDir) {
	var hit = Distance(iXDir, iYDir)/3 - 70;
	if(hit < 1) return;
	if(hit < 90) DoDamage(hit/3);
	else DestroyBlast();
	return;
}

protected func ContainedDigDouble() {
	if(GetAction() == "FreeFall")
		StartLanding();
	return 1;
}

private func StartLanding() {
	if(vertblow) return;
	if(port) port->PortActive(); //Port wird beim laden zweimal aktiviert, falls die Landung früh anfängt...
	SetVertBlowout(1);
	return 1;
}

protected func FxBlowoutTimer(object pObj, int iEffectNumber, int iEffectTime) {
	if(vertblow) {
		var clr = RGBa(200,200,255);
		for(var i = Random(3)+5; i; --i) {
			CreateParticle("Thrust", -Sin(GetR(),8)+Cos(GetR(),-10), Cos(GetR(),8)-Sin(GetR(),11),  -Sin(GetR(),15)+RandomX(-1,1)+GetXDir(), Cos(GetR(),15+Random(10))+GetYDir()/2, 45, clr, this);
			CreateParticle("Thrust", -Sin(GetR(),11),                Cos(GetR(),11),                -Sin(GetR(),15)+RandomX(-1,1)+GetXDir(), Cos(GetR(),15+Random(10))+GetYDir()/2, 45, clr, this);
			CreateParticle("Thrust", -Sin(GetR(),8)+Cos(GetR(),11),  Cos(GetR(),8)-Sin(GetR(),-11), -Sin(GetR(),15)+RandomX(-1,1)+GetXDir(), Cos(GetR(),15+Random(10))+GetYDir()/2, 45, clr, this);
		}
		EffectDust(); //Staub
		if(vertblow == 2) EffectDust();
		//Gegenrotation
		if(GetR() < -1 && GetRDir() < 1) SetRDir(GetRDir(0, 50)+1, 0, 50);
		else if(GetR() > 1 && GetRDir() > -1) SetRDir(GetRDir(0, 50)-1, 0, 50);
		//Beschleunigung
		var accspeed = Min(iCapsMaxSpeed - Cos(GetR()-Angle(0,0,GetXDir(this, 500),GetYDir(this, 500)),Distance(GetYDir(this, 500), GetXDir(this, 500))), iCapsAcceleration+GetGravity());
		if(vertblow != 1 || GetYDir(pObj,500) > iCapsLandSpeed) {
			SetXDir(GetXDir(this, 500)+Sin(GetR(),accspeed), this, 500);
			SetYDir(GetYDir(this, 500)-Cos(GetR(),accspeed), this, 500);
		}
		if(mode && GetY() <= -20 && GetYDir() < 0) {
			for(var pObj in FindObjects(Find_Container(this)))
				if(pObj) pObj -> Sell(GetOwner());
			if(port) port->PortWait();
			RemoveObject();
		}
		if(mode && (iEffectTime > (LandscapeHeight()*4/3 + 300)) && !Random(10)) DoDamage(1); //Falls die Kapsel nicht richtig startet
		
	}
	var acc = 1;
	if(mode) acc = 5; //That's cheating
	if(horrblow == 1) {
		SetXDir(Max(GetXDir(0,70)-acc,-100),0, 70);		
		SetRDir(GetRDir(0, 100)-1, 0, 100);
	} else if(horrblow == -1) {
		SetXDir(Min(GetXDir(0,70)+acc,+100),0, 70);
		SetRDir(GetRDir(0, 100)+1, 0, 100);
	} else if(vertblow == 0 && !mode) { //This means that both are 0 and the capsule is handcontrolled
		return -1;
	}
	if(mode && AdvancedWindCalculations()) {
		var nbo;
		Message("%v %v %v", this, GetXDir(), GetX() - origx, GetWind());
		if(     GetXDir() >= -17 && GetX() > origx) nbo = 1;
		else if(GetXDir() <=  17 && GetX() < origx) nbo = -1;
		if(Abs(GetR()) > 8) {
			if(GetRDir() > 0) SetRDir(GetRDir(0, 100)-1, 0, 100);
			else SetRDir(GetRDir(0, 100)+1, 0, 100);
			nbo = 0;
		}
		if(nbo != horrblow) SetHorrBlowout(nbo);
	}
}

protected func ContactBottom() { 
	if(vertblow == 1) {
		if(mode) {
			RemoveEffect("Blowout", this);
		}
		SetHorrBlowout(0); SetVertBlowout(0); 
		if(port && port==FindObject2(Find_ID(PORT),Find_AtPoint())) {
			portvertex = GetVertexNum();
			AddVertex();
			SetVertex(portvertex, 2, CNAT_NoCollision, this, 1);
			AddVertex();
			SetVertex(portvertex, 0, GetVertex(0, 0, port)+GetX(port)-GetX(), this, 2);
			SetVertex(portvertex, 1, GetVertex(0, 1, port)+GetY(port)-GetY(), this, 2);
			SetAction("PortLand", port);
			SetActionData(256*portvertex, this);
			//FIXME: Do that less hacky...
		} 
		if(ObjectCount2(Find_Container(this), Find_OCF(OCF_CrewMember)))
			ScheduleCall(this, "Eject", 30);
		if(port) ScheduleCall(port, "PortWait", 50);
	}
	return 1;
}

protected func Eject() {
	if(vertblow == 2) return;
	for(var pObj in FindObjects(Find_Container(this), Find_OCF(OCF_CrewMember)))
		pObj -> Exit();
}

protected func ControlUpDouble() {
	if(port) port->PortActive();
	ScheduleCall(this, "Launch", 60);
}

private func Launch() {
	// Remove all bottom vertices to prevent the capsule from stucking.
	for (var vertex in [portvertex, 2, 1, 0])
		RemoveVertex(vertex);
	portvertex = -1;
	SetAction("FreeFall");
	SetActionData();
	SetVertBlowout(2);
	mode = 1;
}

// geklaut von Hazard
protected func EffectDust() {

	// Dust effect
	var mat,i;
	
	// maximum distance in which the shuttle appears
	var maxdistance = 150;
	
	// search for ground (yomisei: please use your sensor-function for that as soon as you finished it)
	for(i=10; i<maxdistance; i+=5) {
		if(GBackSolid(0,i)) {
			mat = GetMaterial(0,i);
			break;
		}
	}
	
	// ground in distance
	if(i<maxdistance) {
	
		// check if digable
		if(CheckDust(mat)) {
		
			// determine material color
			var rand = Random(3);
			var r = GetMaterialColor(mat,rand,0);
			var g = GetMaterialColor(mat,rand,1);
			var b = GetMaterialColor(mat,rand,2);
			
			// all values dependend on distance
			var size = RandomX(20,300-i/2);
			var alpha = Min(255,30+i);
			var pos = RandomX(0,30);
			// the nearer the dust to the center, the faster it is blown aside 
			CreateParticle("BoostDust",-pos,i,(-50+pos)+GetXDir()/2,RandomX(-9,5),size,RGBa(r,g,b,alpha));
			CreateParticle("BoostDust", pos,i,( 50-pos)+GetXDir()/2,RandomX(-9,5),size,RGBa(r,g,b,alpha));
		}
		if(i < 50) {
			var iX, iY, iM = 70 - i, iC = RGB(iM, iM, iM);
			for(var j = 0; j < 5; ++j) {
				iX = RandomX(-20, 20);
				if(!Random(2))
					iX *= 2;
				iY = 0;
				var max = LandscapeHeight() - GetY();
				while(!GBackSolid(iX, ++iY)) {
					if(iY > max)
						return;
				}
				iY += RandomX(-2, 2) + iM / 3;
				if(GBackSolid(iX, iY))
					SetLandscapePixel(iX, iY, iC);
			}
		}
	}
}

public func IsBlowingOut() { return GetEffect("Blowout", this); }

public func Flying() { return !GetContact(this, -1);}

public func MaxDamage() { return 20; }
public func WindEffect() { return 100;}

//
public func ContainedUp(pControl) {
	if(GetPlrCoreJumpAndRunControl(GetOwner(pControl))) {
		SetVertBlowout(1);
	} else {
		SetVertBlowout(BoundBy(vertblow-1,0,2));
		SetHorrBlowout(0);
	}
}

public func ContainedDownDouble(pControl) {return ContainedDown(pControl);}
public func ContainedDown(pControl) {
	if(GetPlrCoreJumpAndRunControl(GetOwner(pControl))) {
		SetVertBlowout(2);
	} else {
		SetVertBlowout(BoundBy(vertblow+1,0,2));
		SetHorrBlowout(0);
	}
	SetCommand(pControl, "None");
}

public func ContainedLeft(pControl) {
	if(GetPlrCoreJumpAndRunControl(GetOwner(pControl))) {
		SetHorrBlowout(-1);
	} else {
		if(horrblow == 1) SetHorrBlowout(0);
		else SetHorrBlowout(-1);
	}
}

public func ContainedRight(pControl) {
	if(GetPlrCoreJumpAndRunControl(GetOwner(pControl))) {
		SetHorrBlowout(1);
	} else {
		if(horrblow == -1) SetHorrBlowout(0);
		else SetHorrBlowout(1);
	}
}

public func ContainedUpReleased() { SetVertBlowout(0); }
public func ContainedDownReleased() { SetVertBlowout(0); }
public func ContainedLeftReleased() { SetHorrBlowout(0); }
public func ContainedRightReleased() { SetHorrBlowout(0); }
public func ContainedDig() { SetHorrBlowout(0); SetVertBlowout(0); }

//Proper Sound() stacking emulator
local sndcount;
private func DoCpslSound(int chng){
	if(sndcount + chng < 1)       Sound("Jetbelt", 0, this, 0, 0, -1);
	if(sndcount == 0 && chng > 0) Sound("Jetbelt", 0, this, 0, 0, 1);
	sndcount += chng;
	if(sndcount < 0) sndcount = 0;
}

protected func ControlDig(object pClonk)
{
  // Herausnehmen per Graben: Holen-Menü öffnen
  pClonk->SetCommand(0, "Get", this, 0,0, 0, 1);
}
