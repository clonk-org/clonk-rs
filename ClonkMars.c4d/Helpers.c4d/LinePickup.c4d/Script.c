/* LinePickup */

#strict 2

private func LinePickup(dummy, object line, bool forced, bool silent)
{
	if (!line) return false;
	// Check physical
	if (!GetPhysical("CanConstruct")) return false;
	var linekit, tstruct;
	tstruct=FindObject2(Find_AtPoint(), Find_OCF(OCF_LineConstruct));
	if (!tstruct) return false;

	if (!forced) { //Bei forced nicht Action ändern
		if (SetAction("Walk"));
			SetSpeed();
		SetComDir(COMD_Stop);
	}

	// Check line pickup
	if (!line->ConnectedTo(tstruct)) return false;
	// Check line connected to linekit at other end
	if (line->ConnectedToLineKit())
	{
		if (!silent) {
			Sound("Error",false,this, 100);
			Message("$TxtNoDoubleKit$", this, line->GetName());
		}
		return false;
	}
	var kitID = line->~LineKitID() || LNKT; //downwards compatible
	// Create new linekit
	if (!(linekit=CreateObject(kitID, 0, 0, line->GetOwner()))) return false;
	// Enter linekit into clonk
	if (!Collect(linekit) && !forced)
	{
		// Enter failed: abort operation
		linekit->RemoveObject(); return false;
	}
	// Attach line to collected linekit
	if (!silent) Sound("Connect",false, this, 100);
	if (line->GetActionTarget(0)==tstruct) line->SetActionTargets(linekit, line->GetActionTarget(1));
	if (line->GetActionTarget(1)==tstruct) line->SetActionTargets(linekit, line->GetActionTarget(0));
	// Message
	if (!silent) Message("$TxtDisconnect$", this, line->GetName(),tstruct->GetName());
	return true;
}

private func CreateLinePickupMenu()
{
	// Check physical
	if (!GetPhysical("CanConstruct")) return false;
	var struct = FindObject2(Find_AtPoint(), Find_OCF(OCF_LineConstruct));
	if (!struct) return false;
	var lines = FindObjects(Find_Action("Connect"), Find_ActionTargets(struct));
	CreateMenu(CXCN, this, this, 0, "$TxtLinePickupCaption$");
	for (var line in lines) {
		if (line->ConnectedToLineKit()) continue;
		if (line->GetActionTarget(1) == struct) { //From me to other
			var other = line->GetActionTarget(0);
			var msg = Format("$TxtPickupLineTo$", GetName(line), GetName(other));
			var dummy = CreateGraphicsDummy(other -> GetID(), line -> GetID());
			AddMenuItem(msg, "LinePickup", 0, this, 0, line, msg, 4, dummy);
			dummy -> RemoveObject();
		}
		else { //From other to me
			var other = line->GetActionTarget(1);
			var msg = Format("$TxtPickupLineFrom$", GetName(line), GetName(other));
			var dummy = CreateGraphicsDummy(other -> GetID(), line -> GetID());
			AddMenuItem(msg, "LinePickup", 0, this, 0, line, msg, 4, dummy);
			dummy -> RemoveObject();
		}
	}
}

private func CreateGraphicsDummy(id obj, id line) {
	var dummy = CreateObject(TIM1);
	SetGraphics(0, dummy, obj, 1, GFXOV_MODE_Picture);
	SetGraphics(0, dummy, line, 2, GFXOV_MODE_Picture);
	return dummy;
}

protected func ContextLinePickup(pCaller)
{
	[$CtxLinePickupDesc$|Image=CXCN|Condition=AtLineToPickup]
	return (CreateLinePickupMenu());
}

protected func AtLineToPickup(pCaller)
{
	var tstruct=FindObject2(Find_AtPoint(), Find_OCF(OCF_LineConstruct));
	if (!tstruct) return false;
	return !! FindObject2(Find_Action("Connect"), Find_ActionTargets(tstruct));
}

global func ConnectedToLineKit(object pObj) {
	if (!pObj) pObj = this;
	if (!pObj) return false;
	if (pObj->GetAction() != "Connect") return false;
	if (pObj->GetActionTarget(0) && pObj->GetActionTarget(0)->~IsLineKit()) return true;
	if (pObj->GetActionTarget(1) && pObj->GetActionTarget(1)->~IsLineKit()) return true;
	return false;
}

global func ConnectedTo(object pTarget, object pObj) {
	if (!pObj) pObj = this;
	if (!pObj) return false;
	if (pObj->GetAction() != "Connect") return false;
	if (pObj->GetActionTarget(0) && pObj->GetActionTarget(0) == pTarget) return true;
	if (pObj->GetActionTarget(1) && pObj->GetActionTarget(1) == pTarget) return true;
	return false;
}
