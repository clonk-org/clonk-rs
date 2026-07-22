#ifndef LC_CONFIG_FFI_H
#define LC_CONFIG_FFI_H

#ifdef __cplusplus
extern "C" {
#else
#include <stdbool.h>
#endif

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

#ifdef __cplusplus
}
#endif

#endif /* LC_CONFIG_FFI_H */
