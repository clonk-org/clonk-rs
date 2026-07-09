/*
 * LegacyClonk
 *
 * Copyright (c) 1998-2000, Matthes Bender (RedWolf Design)
 * Copyright (c) 2017-2026, The LegacyClonk Team and contributors
 *
 * Distributed under the terms of the ISC license; see accompanying file
 * "COPYING" for details.
 */

#pragma once

#include <cstdint>

namespace C4ActionDirection
{
	enum class Horizontal : int32_t
	{
		None = 0,
		Left = -1,
		Right = 1,
	};

	struct HorizontalUpdate
	{
		Horizontal Direction;
		int32_t PhaseAdvance;
	};

	// C4Object::ExecAction derives both animation speed and facing from the
	// sign of the raw C4Fixed xdir. Keep this decision independent of any
	// rounded integer velocity mirror.
	template<typename Fixed>
	constexpr HorizontalUpdate FromHorizontalVelocity(const Fixed &xdir, int32_t phaseScale)
	{
		if (xdir < 0) return {Horizontal::Left, -fixtoi(xdir * phaseScale)};
		if (xdir > 0) return {Horizontal::Right, +fixtoi(xdir * phaseScale)};
		return {Horizontal::None, 0};
	}

	constexpr bool RunsTurnAction(int32_t currentDirection, int32_t requestedDirection, bool hasTurnAction)
	{
		return requestedDirection != currentDirection && hasTurnAction;
	}
}
