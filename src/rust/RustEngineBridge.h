#pragma once

#include <cstdint>
#include <cstddef>

class C4Game;
class C4Control;

#ifdef USE_RUST_ENGINE_VALIDATION

namespace RustEngineBridge {

void OnFrame(C4Game &game);
void OnGameStart(C4Game &game);
void OnControlFrame(const C4Control &control, uint64_t frame);
void OnNetworkPacket(
    uint8_t status,
    const uint8_t *payload,
    size_t payload_size,
    int32_t client_id,
    uint32_t connection_id,
    bool inbound);
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
inline void OnNetworkPacket(
    uint8_t,
    const uint8_t *,
    size_t,
    int32_t,
    uint32_t,
    bool) {}

} // namespace RustEngineBridge

#endif
