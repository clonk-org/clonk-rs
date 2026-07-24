/* PipeHead */

#strict 2

#appendto PIPH

private func FindDerrick() {
	var pObj = FindObject2(Find_ID(RIG_), Find_ActionTarget(this), Find_Or(Find_Action("Pump"), Find_Action("Wait")));
	return pObj;
}

local HasEnergy;

// Returns the number of pixels to transfer in one go.
private func TransferAmount(int iMat) {
	if(MaterialName(iMat) == "Oil") {
		return 3;
	} else {
		// Lava
		return 9;
	}
}

protected func Transfer() {
	var pTarget = GetActionTarget(), iExtracted;
	if (!pTarget) return; //Bohrturm verschwunden?
	var mat = GetMaterial();
	var amount = TransferAmount(mat);
	// Passendes Ziel mit genügend Platz vorhanden?
	if(!(pTarget -> Transfer(amount, mat, true)))
		return StopTransfer(pTarget);
	
	if(!HasEnergy || GetActTime() % 10) {
		if(pTarget -> CheckPower(5))
			HasEnergy = true;
		else {
			HasEnergy = false;
			return StopTransfer(pTarget);
		}
	}
	
	// in einem Durchgang wird nur ein Materialtyp extrahiert
	for(var i = amount; i; i--) {
		if(ExtractLiquid() == mat)
			iExtracted++;
		else
			break;
	}
	if(pTarget -> GetAction() != "Pump")
		pTarget -> SetAction("Pump", this);
	pTarget -> Transfer(iExtracted, mat);
}

private func StopTransfer(object pTarget) {
	if(pTarget -> GetAction() != "Wait")
		pTarget -> SetAction("Wait", this);
}
