/* Meteor-Append für RADO */

#strict 2
#appendto METO

protected func Initialize() {
	AddEffect("RADO", this, 10, 1, this);
	return _inherited(...);
}

protected func FxRADOTimer() {
	for(var rado in FindObjects(Find_ID(RADO), Find_Distance(RADO_Distance), Sort_Random())) {
		if(rado->DoBlast(false))
			return;
	}
}
