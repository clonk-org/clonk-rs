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

namespace C4ActionCallbacks
{
	enum class Kind
	{
		Start,
		End,
		Abort,
	};

	// C4Object::SetAction issues callbacks synchronously in this exact order.
	// Returning false from a callback aborts the remaining sequence, e.g. when
	// a script callback changed the object's definition or removed the object.
	template<typename Callback>
	constexpr bool Dispatch(bool startRequested, bool endRequested, bool abortRequested,
		bool forced, bool oldActionActive, bool newActionActive, Callback &&callback)
	{
		if (startRequested && newActionActive && !callback(Kind::Start)) return false;
		if (endRequested && !forced && oldActionActive && !callback(Kind::End)) return false;
		if (abortRequested && !forced && oldActionActive && !callback(Kind::Abort)) return false;
		return true;
	}
}
