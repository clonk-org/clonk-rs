/*
 * LegacyClonk
 *
 * Copyright (c) 2026, The LegacyClonk Team and contributors
 *
 * Distributed under the terms of the ISC license; see accompanying file
 * "COPYING" for details.
 *
 * "Clonk" is a registered trademark of Matthes Bender, used with permission.
 * See accompanying file "TRADEMARK".
 *
 * To redistribute this file separately, substitute the full license texts
 * for the above references.
 */

#pragma once

#include <C4Constants.h>

#include <cstdint>

namespace C4ScriptKiller
{
	template<typename Object>
	int32_t Get(Object *pContextObject, Object *pObject)
	{
		if (!pObject) pObject = pContextObject;
		if (!pObject) return NO_OWNER;
		return pObject->LastEnergyLossCausePlayer;
	}

	template<typename Object, typename ValidPlayer>
	bool Set(int32_t iNewKiller, Object *pContextObject, Object *pObject, ValidPlayer ValidPlr)
	{
		if (iNewKiller != NO_OWNER && !ValidPlr(iNewKiller)) return false;
		if (!pObject) pObject = pContextObject;
		if (!pObject) return false;
		pObject->LastEnergyLossCausePlayer = iNewKiller;
		return true;
	}
}
