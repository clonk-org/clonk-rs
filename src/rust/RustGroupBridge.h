#pragma once

class C4Group;

namespace RustGroupBridge {
#ifdef USE_RUST_GROUP_VALIDATION
void ValidateOnOpen(C4Group &group);
#else
inline void ValidateOnOpen(C4Group &) {}
#endif
} // namespace RustGroupBridge
