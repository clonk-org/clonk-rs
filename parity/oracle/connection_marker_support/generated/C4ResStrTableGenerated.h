#pragma once

#include <array>
#include <cstddef>

// The archived oracle omits the build-generated resource table. The bounded
// packet proof includes C4Log.h through C4Network2IO.h but never references a
// resource key, so a one-entry declaration is sufficient to compile those
// production declarations without linking the engine/resource subsystem.
enum class C4ResStrTableKey : std::size_t
{
    NumberOfEntries = 1,
};

inline constexpr std::array<std::size_t, 1> C4ResStrTableKeyFormatStringArgsCount{{0}};
