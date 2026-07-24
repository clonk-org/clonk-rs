//RandomX(-2,2),100,150);
/*-- Booster --*/

#strict 2

local countdown;

public func StartCountdown() {
	AddEffect("Countdown", this, 123, 36,this);
	//debug
	countdown = 110;
}

//Countdown
protected func FxCountdownTimer(){
	//bei 2 min, 1 min und 30 sekunden Zeit ausgeben
	if(countdown == 0 || countdown == 60 || countdown == 90){
		Log("$rocketStarts$ %s $rocketStarts2$ %d $seconds$",GetPlayerName(GetOwner(this)),120-countdown);
		//Sound(Format("t_%d",120-cSound("Rocket_engineloop",0,this,100,0,1);ountdown));
	}
	//noch 30s
	if(countdown == 90){
		Sound("Rocket_pressure",0,this,0,0,1);
	}
	//noch 10s
	if(countdown == 110){
		Sound("Rocket_pressure",0,this,0,0,-1);
		Sound("Rocket_sparkle_loop",0,this,0,0,1);
	}
	//noch 4s
	if(countdown == 116){
		Sound("Rocket_sparkle_loop",0,this,0,0,-1);
		Sound("Rocket_enginestart",0,this,0,0,0);
		AddEffect("Launch", this, 123, 1,this);
	}
	//noch 2s
	if(countdown == 118){
		LaunchEarthquake(0, 40,500);//Erdbeben
	}
	//Ab 10s Countdown
	if(countdown >= 110){
		Log("%d",120-countdown);
		//Sound(Format("t_%d",120-countdown));
	}
	//Nach 2 Minuten Countdown löschen
	if(countdown == 120){
		RemoveEffect("Countdown",this);
	}
	//Zeit erhöhen
	countdown++;
}

//Ende des Countdowns
protected func FxCountdownStop(){
	SetYDir(GetYDir(this)-1);
}

//Tolle Effekte
protected func FxLaunchTimer(){
	//generelle Effekte
	
	//Enginestart - noch nicht bewegen
	if(GetEffect("Launch",this,0,6) < 300){
		for(var i = 0; i < 15; i++){
			//motoren an - aber noch nicht abheben
		}
		for(var i = 0; i < 3; i++){
			CreateParticle("Smoke1",0,35,RandomX(-50,50),RandomX(-10,20),RandomX(200,400));
			CreateParticle("Smoke1",11,35,RandomX(-50,50),RandomX(-10,20),RandomX(100,400));
			CreateParticle("Smoke1",-11,35,RandomX(-50,50),RandomX(-10,20),RandomX(100,400));
		}
	}
	if(GetEffect("Launch",this,0,6) > 144){
		if(GetEffect("Launch",this,0,6) == 144){
			SetYDir(-2);
		}
		/*if(GetYDir(this) > -(GetEffect("Launch",this,0,6)/36)){
			SetYDir(-(1/100 * GetYDir(this) * GetYDir(this))+ 2 * GetYDir(this));
		}*/
		SetYDir(-GetEffect("Launch",this,0,6)/20 + 5);
		Sound("Rocket_engineloop",0,this,100,0,0);
		for(var i = 0; i < 15; i++){
			CreateParticle("Thrust",0,35,RandomX(-2,2),100,180,RGBa(255,50,0,0));
			CreateParticle("Thrust",10,35,RandomX(-1,1),40,100,RGBa(255,100,40,0));
			CreateParticle("Thrust",-10,35,RandomX(-1,1),60,100,RGBa(255,100,40,0));
		}
		for(var i = 0; i < 5; i++){
			CreateParticle("Smoke2",0,65,RandomX(-2,2),80,150);
			CreateParticle("Smoke2",11,65,RandomX(-2,2),70,70);
			CreateParticle("Smoke2",-11,65,RandomX(-2,2),70,70);
			
			CreateParticle("Smoke2",0,55,RandomX(-50,50),RandomX(50,200),RandomX(200,400));
			CreateParticle("Smoke2",0,55,RandomX(-50,50),RandomX(50,200),RandomX(200,400));
			CreateParticle("Smoke2",0,55,RandomX(-50,50),RandomX(50,200),RandomX(200,400));
			CreateParticle("Smoke2",0,55,RandomX(-50,50),RandomX(50,200),RandomX(200,400));
		}
	}
	if(GetY(this) < -400){
		RemoveObject(this);
	}
}

public func MarsResearch() {
	return true;
}

