/*
 * LegacyClonk
 *
 * Copyright (c) 1998-2000, Matthes Bender (RedWolf Design)
 * Copyright (c) 2017-2021, The LegacyClonk Team and contributors
 *
 * Distributed under the terms of the ISC license; see accompanying file
 * "COPYING" for details.
 */

#pragma once

#include <cstdint>

namespace C4LandscapePath
{
	template<typename IsOccupied>
	bool IsFree(int32_t x, int32_t y, int32_t x2, int32_t y2, IsOccupied isOccupied)
	{
		x /= 17; y /= 15; x2 /= 17; y2 /= 15;
		while (x != x2 && y != y2)
		{
			if (isOccupied(x, y)) return false;
			if (x > x2) x--; else x++;
			if (y > y2) y--; else y++;
		}
		if (x != x2)
			do
			{
				if (isOccupied(x, y)) return false;
				if (x > x2) x--; else x++;
			} while (x != x2);
		else
			while (y != y2)
			{
				if (isOccupied(x, y)) return false;
				if (y > y2) y--; else y++;
			}
		return !isOccupied(x, y2);
	}
}
