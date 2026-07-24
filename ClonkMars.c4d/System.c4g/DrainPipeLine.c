/* Pipeline */

#strict 2

#appendto DPIP

protected func Initialize() {
	SetName("$Pipe$", 0, GetID());
	return _inherited(...);
}

public func Transfer() {
	return;
}

public func LineKitID() { return PIKT; }
