# Using the Rust Config Loader from C++

Include `rust/include/lc_config_ffi.h` (installed with the Rust artefact) which exposes the ABI:

```c
typedef struct ConfigHandle ConfigHandle;

ConfigHandle *lc_config_load(const char *path);
void lc_config_free(ConfigHandle *handle);
const char *lc_config_get_value(ConfigHandle *handle, const char *key);
const char *lc_config_get_value_in(ConfigHandle *handle, const char *section, const char *key);
const char *lc_config_dump(ConfigHandle *handle);
const char *lc_config_compare_with_dump(ConfigHandle *handle, const char *legacy_dump);
bool lc_config_replace_from_text(ConfigHandle *handle, const char *text);
bool lc_config_save(ConfigHandle *handle, const char *path);
void lc_string_free(char *value);
```

## Ownership Rules
- `lc_config_load` returns `nullptr` on failure and a heap-allocated handle otherwise; call `lc_config_free` exactly once.
- `lc_config_get_value`, `lc_config_get_value_in`, `lc_config_dump`, and `lc_config_compare_with_dump` return freshly allocated C strings. Always release them with `lc_string_free` after use.
- `lc_config_replace_from_text` re-parses a complete INI string and replaces the Rust config state, returning `false` on parse errors.
- `lc_config_save` writes the current Rust config to the supplied path and reports success via its boolean return value.
- Passing `nullptr` as the `section` argument treats the lookup as global (no section).

## Feature Flag Recommendation
Wrap the FFI usage behind a build flag (e.g. `USE_RUST_CONFIG`) so you can run the legacy and Rust loaders side-by-side during parity checks.

## Error Handling
When a key (or section/key pair) is missing, the getter returns `nullptr` without logging. The C++ caller should fall back to default values and optionally emit diagnostics for parity comparison. The comparison helper returns `nullptr` on parity success and a newline-delimited report when differences are detected. Replacements and saves return `false` on failure so callers can fall back to legacy behaviour.
