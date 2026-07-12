#pragma once

#include <cstdint>
#include <cstddef>

class C4Game;
class C4Control;

#ifdef USE_RUST_ENGINE_VALIDATION

namespace RustEngineBridge {

int RunControlCodecOracle(int argc, char *const argv[]);
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
bool FindPath(
    C4Game &game,
    int32_t from_x,
    int32_t from_y,
    int32_t to_x,
    int32_t to_y,
    bool transfer_zones_enabled,
    int32_t level,
    bool (*set_waypoint)(int32_t, int32_t, intptr_t, intptr_t),
    intptr_t parameter);

} // namespace RustEngineBridge

#else

namespace RustEngineBridge {

inline int RunControlCodecOracle(int, char *const[]) { return -1; }
inline void OnFrame(C4Game &) {}
inline void OnGameStart(C4Game &) {}
inline void OnControlFrame(const C4Control &, uint64_t) {}
inline void Shutdown() {}
inline bool IsActive() { return false; }
inline bool FindPath(
    C4Game &game,
    int32_t from_x,
    int32_t from_y,
    int32_t to_x,
    int32_t to_y,
    bool transfer_zones_enabled,
    int32_t level,
    bool (*set_waypoint)(int32_t, int32_t, intptr_t, intptr_t),
    intptr_t parameter) {
    (void)game;
    (void)from_x;
    (void)from_y;
    (void)to_x;
    (void)to_y;
    (void)transfer_zones_enabled;
    (void)level;
    (void)set_waypoint;
    (void)parameter;
    return false;
}
inline void OnNetworkPacket(
    uint8_t,
    const uint8_t *,
    size_t,
    int32_t,
    uint32_t,
    bool) {}

} // namespace RustEngineBridge

#endif
