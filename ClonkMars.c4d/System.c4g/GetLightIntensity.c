/* Gibt die aktuelle Helligkeit zurück */

#strict 2

global func GetLightIntensity() {
	var pTime = FindObject2(Find_ID(TIME));
	if(!pTime)
		return 100; // volle Helligkeit
	return pTime -> Local(2);
}
