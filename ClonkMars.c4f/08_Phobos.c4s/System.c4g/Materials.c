/* gräuliche Materialien */

#strict 2

#appendto ROCK
#appendto ERTH

protected func Initialize() {
	SetClrModulation(RGB(1, 1, 1));
	return _inherited(...);
}
