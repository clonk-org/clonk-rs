#strict 2

/* Checkt ob hier ein Dust-Partikel erstellt werden darf */

global func CheckDust(int mat) {

		// check if digable
		var digable = GetMaterialVal("DigFree","Material",mat);
		
		// hack for not-digable materials but should be by name digable
		if(!digable) {
			var name = GetMaterialVal("Name","Material",mat);
			if(WildcardMatch(name, "*Earth")) digable = true;
			else if(WildcardMatch(name, "*Sand")) digable = true;
			else if(WildcardMatch(name, "*Ashes")) digable = true;
			else if(WildcardMatch(name, "*Snow")) digable = true;
		}
		
		return digable;
}
