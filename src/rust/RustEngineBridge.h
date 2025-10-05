#pragma once

class C4Game;

#ifdef USE_RUST_ENGINE_VALIDATION

namespace RustEngineBridge {

void OnFrame(C4Game &game);
void Shutdown();
bool IsActive();

} // namespace RustEngineBridge

#else

namespace RustEngineBridge {

inline void OnFrame(C4Game &) {}
inline void Shutdown() {}
inline bool IsActive() { return false; }

} // namespace RustEngineBridge

#endif
