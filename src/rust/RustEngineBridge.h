#pragma once

#include <cstdint>

class C4Game;
class C4Control;

#ifdef USE_RUST_ENGINE_VALIDATION

namespace RustEngineBridge {

void OnFrame(C4Game &game);
void OnGameStart(C4Game &game);
void OnControlFrame(const C4Control &control, uint64_t frame);
void Shutdown();
bool IsActive();

} // namespace RustEngineBridge

#else

namespace RustEngineBridge {

inline void OnFrame(C4Game &) {}
inline void OnGameStart(C4Game &) {}
inline void OnControlFrame(const C4Control &, uint64_t) {}
inline void Shutdown() {}
inline bool IsActive() { return false; }

} // namespace RustEngineBridge

#endif
