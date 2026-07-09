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

namespace C4SolidMaskBitmap
{
	template<typename Object>
	auto *GetActiveBitmap(Object *object)
	{
		return object->GetGraphics()->GetBitmap();
	}

	template<typename Bitmap>
	uint8_t MaskPixel(Bitmap *bitmap, int32_t x, int32_t y)
	{
		return bitmap->IsPixTransparent(x, y) ? 0x00 : 0xff;
	}
}
