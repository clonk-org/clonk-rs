#ifndef LC_GROUP_FFI_H
#define LC_GROUP_FFI_H

#ifdef __cplusplus
extern "C" {
#else
#include <stdbool.h>
#endif

#include <stddef.h>
#include <stdint.h>

typedef struct GroupHandle GroupHandle;

typedef struct LcGroupEntry {
    char *path;
    bool is_directory;
    uint64_t size;
} LcGroupEntry;

GroupHandle *lc_group_open(const char *path);
void lc_group_free(GroupHandle *handle);
LcGroupEntry *lc_group_entries(GroupHandle *handle, size_t *len);
void lc_group_entries_free(LcGroupEntry *entries, size_t len);
unsigned char *lc_group_read_file(GroupHandle *handle, const char *relative_path, size_t *len);
void lc_group_buffer_free(unsigned char *buffer, size_t len);
bool lc_group_exists(GroupHandle *handle, const char *relative_path);
char *lc_group_maker(GroupHandle *handle);
char *lc_group_root(GroupHandle *handle);
void lc_group_string_free(char *value);

#ifdef __cplusplus
}
#endif

#endif /* LC_GROUP_FFI_H */
