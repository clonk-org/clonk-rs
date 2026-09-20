#include "C4Group.h"
#include "RustGroupBridge.h"
#include "lc_group_ffi.h"

#include <cstdlib>
#include <iostream>

// These fixtures perturb only the diagnostic bridge's FFI boundary. They do
// not replace the parser or any simulation code in the built oracle.
struct GroupHandle {};
static std::vector<ProbeEntry> rust_entries;
static std::size_t allocated_count = 0;
static bool wrong_free_count = false;
static int handles = 0;

extern "C" GroupHandle *lc_group_open(const char *) {
    ++handles;
    return new GroupHandle;
}
extern "C" void lc_group_free(GroupHandle *handle) {
    --handles;
    delete handle;
}
extern "C" LcGroupEntry *lc_group_entries(GroupHandle *, std::size_t *length) {
    *length = allocated_count = rust_entries.size();
    auto *rows = new LcGroupEntry[*length];
    for (std::size_t index = 0; index < *length; ++index) {
        const auto &entry = rust_entries[index];
        rows[index] = {strdup(entry.name.c_str()), entry.directory, entry.size};
    }
    return rows;
}
extern "C" void lc_group_entries_free(LcGroupEntry *rows, std::size_t length) {
    wrong_free_count = length != allocated_count;
    for (std::size_t index = 0; index < allocated_count; ++index) std::free(rows[index].path);
    delete[] rows;
}
extern "C" unsigned char *lc_group_read_file(GroupHandle *, const char *name, std::size_t *length) {
    for (const auto &entry : rust_entries) {
        if (entry.name == name && !entry.directory) {
            *length = entry.size;
            return new unsigned char[*length]{};
        }
    }
    *length = 0;
    return nullptr;
}
extern "C" void lc_group_buffer_free(unsigned char *buffer, std::size_t) { delete[] buffer; }
extern "C" bool lc_group_exists(GroupHandle *, const char *name) {
    for (const auto &entry : rust_entries) if (entry.name == name) return true;
    return false;
}
extern "C" char *lc_group_maker(GroupHandle *) { return strdup("fixture-maker"); }
extern "C" char *lc_group_root(GroupHandle *) { return strdup("fixture.c4g"); }
extern "C" void lc_group_string_free(char *value) { std::free(value); }

int main(int argc, char **argv) {
    unsetenv("LC_RUST_GROUP_FAULT");
    unsetenv("LC_RUST_GROUP_DEEP");
    C4Group group;
    rust_entries = group.entries;
    const std::string fixture = argc > 1 ? argv[1] : "agree";
    if (fixture == "missing") rust_entries.pop_back();
    else if (fixture == "additional") rust_entries.push_back({"Extra.txt", 1, false});
    else if (fixture == "size") ++rust_entries[0].size;
    else if (fixture == "type") rust_entries[0].directory = true;
    else if (fixture == "canonical") {
        group.entries[0] = {"Nested\\Alpha.txt", 5, false};
        rust_entries[0] = {"Nested/Alpha.txt", 5, false};
        group.entries[1] = {"Child/", 0, true};
        rust_entries[1] = {"Child", 0, true};
    }
    else if (fixture != "agree") return 2;
    RustGroupBridge::ValidateOnOpen(group);
    if (wrong_free_count || handles != 0) {
        std::cerr << "the bridge did not release the complete allocation\n";
        return 1;
    }
}
