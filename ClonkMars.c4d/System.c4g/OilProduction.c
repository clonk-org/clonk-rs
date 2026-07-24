/* Oil production: Only require 80% of oil. */

#strict 2

#appendto OILP

local targetMatCount;

private func GetMaterialCount() {
	var result = inherited(...);
	if (result != -1) {
		if (!targetMatCount) {
			targetMatCount = result * 20 / 100;
		}
		result -= targetMatCount;
	}
	return result;
}
