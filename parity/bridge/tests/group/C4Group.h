#pragma once

// Fixture boundary for the pinned diagnostic bridge. This deliberately models
// only the entry enumeration API; the live oracle run covers C4Group itself.
#include <cstddef>
#include <cstring>
#include <string>
#include <vector>

constexpr int GRPF_File = 1;
constexpr int GRPF_Folder = 2;

struct ProbeEntry {
    std::string name;
    std::size_t size;
    bool directory;
};

struct ProbeName {
    const char *getData() const { return "fixture.c4g"; }
};

class StdBuf {
public:
    std::vector<unsigned char> data;
    std::size_t getSize() const { return data.size(); }
    const void *getData() const { return data.data(); }
};

class C4Group {
public:
    std::vector<ProbeEntry> entries{{"Alpha.txt", 5, false}, {"Beta.txt", 4, false}};
    int status = GRPF_File;
    C4Group *GetMother() const { return nullptr; }
    int GetStatus() const { return status; }
    bool IsPacked() const { return status == GRPF_File; }
    const char *GetMaker() const { return "fixture-maker"; }
    bool LoadEntry(const char *name, StdBuf &buffer) {
        for (const auto &entry : entries) {
            if (entry.name == name && !entry.directory) {
                buffer.data.assign(entry.size, 0);
                return true;
            }
        }
        return false;
    }
    ProbeName GetFullName() const { return {}; }
    void ResetSearch() { cursor = 0; }
    bool FindEntry(const char *pattern, char *name, std::size_t *size, bool *child) {
        ResetSearch();
        return FindNextEntry(pattern, name, size, child);
    }
    bool FindNextEntry(const char *, char *name, std::size_t *size, bool *child) {
        if (cursor == entries.size()) return false;
        const auto &entry = entries[cursor++];
        std::strcpy(name, entry.name.c_str());
        *size = entry.size;
        *child = entry.directory;
        return true;
    }
private:
    std::size_t cursor = 0;
};
