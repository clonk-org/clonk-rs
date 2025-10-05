# RustConfigBridge Usage

With `USE_RUST_CONFIG` defined, include `src/rust/RustConfigBridge.h` to access the Rust-backed loader:

```cpp
#include "rust/RustConfigBridge.h"

if (RustConfigBridge::LoadConfig(configPath)) {
    if (auto optValue = RustConfigBridge::GetValueIn("Graphics", "Engine")) {
        // use *optValue
    }
}
```

Remember to call `RustConfigBridge::Unload()` during shutdown. When the flag is disabled, the bridge functions no-op and return `std::nullopt`, allowing seamless toggling.

When you want to persist updated settings, serialize the current `C4Config` via `StdCompilerINIWrite`, feed the dump back through `RustConfigBridge::ReplaceFromText`, and then invoke `RustConfigBridge::SaveConfig` so the Rust side emits the file while staying in sync with C++ (the engine exposes `C4Config::SyncRust()` to wrap this sequence when parity is active).

To compare with the legacy loader, load via both paths and diff key/value pairs, logging discrepancies before switching the default.
