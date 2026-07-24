/*-- O2 generator --*/

#strict 2

#include L_SS

/*-- Public calls --*/
public func MaxFill_O2() 
{
	return 0;
}

// This object is a power generator.
public func IsO2Producer()
{
	return GetO2() >= 10;
}

public func PipelineConnect() {
	return 1;
}

// Returns the generator's priority, consumers preferably drain from generators with the highest priority.
public func GetO2GeneratorPriority()
{
	return 1;
}

/*-- O2 generation --*/

// Verbrauch durch Clonk
public func DrainO2() {
	return DoO2(-10);
}

public func & GetO2()
{
	return GetFill("O2");
}

public func DoO2(int iDo)
{
  return DoFill(iDo, "O2");
}

