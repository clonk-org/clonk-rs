/* Pseudo-random number generator */

#strict 2

static _prandom_seed;

global func _PRandom(int a) {
	return (a * 134775813 + 1) % 2147483647;
}

global func PRandom(int max) {
	if (!_prandom_seed) {
		_prandom_seed = 1;
	}
	_prandom_seed = _PRandom(_prandom_seed);
	return Abs(_prandom_seed % max);
}

global func PRandomX(int iStart, int iEnd) {
	var iSwap;
	// Werte vertauscht: trotzdem richtig rechnen
	if(iStart > iEnd) { iSwap=iStart; iStart=iEnd; iEnd=iSwap; }
	// Zufälligen Faktor bestimmen (Differenz plus die null) und iStart aufrechnen
	return PRandom(iEnd-iStart+1)+iStart;
}
