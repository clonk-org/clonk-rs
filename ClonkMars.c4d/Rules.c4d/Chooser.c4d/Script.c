/*-- Spielzielauswahl --*/

#strict 2

local choosable;

protected func Initialize() {
	// Don't allow choosing rules when playing in the league.
	if(GetLeague()) {
		RemoveObject();
	}
}

protected func Activate(int iPlr) {
	if(!iPlr && GetCursor()) {
		OpenMenu();
	}
	else {
		MessageWindow(GetDesc(), iPlr);
	}
}

private func OpenMenu() {
	var menu = CreateMenuTemplate(GetID(), "$RuleInfo$");
	
	var rules = GameCall("ChoosableRules");
	if(!rules) {
		rules = [];
		for(var i = 0, ID; ID = GetDefinition(i, C4D_Rule); i++) {
			if(ID -> ~IsChoosable())
				PushBack(ID, rules);
		}
	}
	
	var antirules = GameCall("NotChoosableRules");
	choosable = [];
	
	for(var ID in rules) {
		if(GetIndexOf(ID, antirules) != -1)
			continue;
		PushBack(ID, choosable);
		
		var condition = ID -> ~ChoosingCondition();
		AddBoolChoice(menu, 0, ID, condition, GetName(0, ID), ID, ObjectCount2(Find_ID(ID)));
	}
	
	CreateMenuByTemplate(GetCursor(), this, "Choose", menu);
}

public func Choose(array rules, ExtraData, bool canceled) {
	// bei Schlieﬂen durch Graben nichts tun
	if(canceled)
		return;
	
	// Sound!
	Sound("Special_Bip", true);
	
	// alle w‰hlbaren Regeln entfernen
	for(var ID in choosable) {
		RemoveAll(ID);
	}
	
	var result = CreateArray();
	
	var iter = HashIter(rules), node;
	while(node = HashIterNext(iter)) {
		if(node[1]) {
			CreateObject(node[0], 0, 0, NO_OWNER);
			PushBack(node[0], result);
		}
	}
	
	var lastItem = result[GetLength(result) - 1];
	SetLength(result, GetLength(result) - 1);
	
	var output = "Regeln: ";
	
	for(var ID in result) {
		output = Format("%s%s, ", output, GetName(0, ID));
	}
	
	output = Format("%s%s", output, GetName(0, lastItem));
	
	Log(output);
	Message(output);
	
	RemoveObject();
}
