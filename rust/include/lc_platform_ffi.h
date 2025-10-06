#ifndef LC_PLATFORM_FFI_H
#define LC_PLATFORM_FFI_H

#ifdef __cplusplus
extern "C" {
#endif

char *lc_platform_install_root(void);
char *lc_platform_system_group_path(void);
char *lc_platform_user_data_dir(void);
char *lc_platform_cache_dir(void);
char *lc_platform_logs_dir(void);
char *lc_platform_temp_dir(void);
char *lc_platform_config_dir(void);
void lc_platform_string_free(char *value);

#ifdef __cplusplus
}
#endif

#endif // LC_PLATFORM_FFI_H
