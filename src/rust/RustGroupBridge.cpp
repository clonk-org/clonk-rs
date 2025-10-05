#include "RustGroupBridge.h"

#ifdef USE_RUST_GROUP_VALIDATION

#include "lc_group_ffi.h"

#include <C4Group.h>
#ifdef C4ENGINE
#include "C4Log.h"
#endif
#include <Standard.h>
#include <StdFile.h>

#include <algorithm>
#include <array>
#include <string>
#include <unordered_map>
#include <vector>

namespace {

struct GroupHandlePtr {
    explicit GroupHandlePtr(GroupHandle *raw) : handle(raw) {}
    GroupHandlePtr(const GroupHandlePtr &) = delete;
    GroupHandlePtr &operator=(const GroupHandlePtr &) = delete;
    GroupHandlePtr(GroupHandlePtr &&other) noexcept : handle(other.handle) { other.handle = nullptr; }
    GroupHandlePtr &operator=(GroupHandlePtr &&other) noexcept {
        if (this != &other) {
            if (handle) {
                lc_group_free(handle);
            }
            handle = other.handle;
            other.handle = nullptr;
        }
        return *this;
    }
    ~GroupHandlePtr() {
        if (handle) {
            lc_group_free(handle);
        }
    }
    GroupHandle *get() const { return handle; }
    explicit operator bool() const { return handle != nullptr; }

private:
    GroupHandle *handle;
};

struct EntryArrayPtr {
    EntryArrayPtr(LcGroupEntry *raw, std::size_t count) : entries(raw), len(count) {}
    EntryArrayPtr(const EntryArrayPtr &) = delete;
    EntryArrayPtr &operator=(const EntryArrayPtr &) = delete;
    ~EntryArrayPtr() {
        if (entries) {
            lc_group_entries_free(entries, len);
        }
    }
    LcGroupEntry *get() const { return entries; }
    std::size_t size() const { return len; }

private:
    LcGroupEntry *entries;
    std::size_t len;
};

std::string Canonicalise(std::string value) {
    std::replace(value.begin(), value.end(), '\\', '/');
    while (!value.empty() && value.back() == '/') {
        value.pop_back();
    }
    return value;
}

bool DetermineIsDirectory(C4Group &group, const std::string &name, bool reported_child) {
    if (group.IsPacked()) {
        return reported_child;
    }

    char path[_MAX_PATH + 1];
    SCopy(group.GetFullName().getData(), path, _MAX_PATH);
    AppendBackslash(path);
    SAppend(name.c_str(), path, _MAX_PATH);
    return DirectoryExists(path);
}

struct LegacyEntry {
    std::string original;
    std::string canonical;
    uint64_t size;
    bool is_directory;
};

std::vector<LegacyEntry> CollectLegacyEntries(C4Group &group) {
    std::vector<LegacyEntry> entries;
    char name[_MAX_FNAME + 1]{};
    size_t size = 0;
    bool child = false;

    if (!group.FindEntry("*", name, &size, &child)) {
        group.ResetSearch();
        return entries;
    }

    do {
        std::string original{name};
        std::string canonical = Canonicalise(original);
        const bool is_directory = DetermineIsDirectory(group, original, child);
        entries.push_back({std::move(original), std::move(canonical), static_cast<uint64_t>(size), is_directory});
    } while (group.FindNextEntry("*", name, &size, &child));

    group.ResetSearch();
    return entries;
}

struct RustEntry {
    std::string original;
    std::string canonical;
    uint64_t size;
    bool is_directory;
};

std::string SummariseList(const std::vector<std::string> &items) {
    if (items.empty()) {
        return {};
    }
    const std::size_t max_items = 5;
    std::string summary;
    for (std::size_t i = 0; i < items.size() && i < max_items; ++i) {
        if (!summary.empty()) {
            summary += ", ";
        }
        summary += items[i];
    }
    if (items.size() > max_items) {
        summary += ", ...";
    }
    return summary;
}

} // namespace

namespace RustGroupBridge {

void ValidateOnOpen(C4Group &group) {
    if (group.GetMother()) {
        return; // Only validate top-level groups backed by on-disk resources.
    }

    const int status = group.GetStatus();
    if (status != GRPF_File && status != GRPF_Folder) {
        return;
    }

    const auto full_name = group.GetFullName();
    if (!full_name.getData()[0]) {
        return;
    }

    GroupHandlePtr handle(lc_group_open(full_name.getData()));
    if (!handle) {
#ifdef C4ENGINE
        LogNTr(spdlog::level::warn, "Rust group validation failed: could not open {}", full_name.getData());
#endif
        return;
    }

    std::size_t len = 0;
    EntryArrayPtr ffi_entries(lc_group_entries(handle.get(), &len), len);

    std::unordered_map<std::string, RustEntry> rust_entries;
    if (auto raw = ffi_entries.get()) {
        rust_entries.reserve(len);
        for (std::size_t i = 0; i < len; ++i) {
            const auto &entry = raw[i];
            if (!entry.path) {
                continue;
            }
            std::string original(entry.path);
            std::string canonical = Canonicalise(original);
            rust_entries.emplace(canonical, RustEntry{std::move(original), std::move(canonical), entry.size, entry.is_directory});
        }
    }

    auto legacy_entries = CollectLegacyEntries(group);

    std::vector<std::string> missing_in_rust;
    std::vector<std::string> missing_in_legacy;
    std::vector<std::string> size_mismatches;
    std::vector<std::string> type_mismatches;

    for (const auto &legacy : legacy_entries) {
        const auto it = rust_entries.find(legacy.canonical);
        if (it == rust_entries.end()) {
            missing_in_rust.push_back(legacy.original);
            continue;
        }

        const RustEntry &rust_entry = it->second;
        const bool comparable_sizes = !legacy.is_directory && !rust_entry.is_directory;
        if (comparable_sizes && legacy.size != rust_entry.size) {
            size_mismatches.push_back(legacy.original);
        }

        if (status == GRPF_File && legacy.is_directory != rust_entry.is_directory) {
            type_mismatches.push_back(legacy.original);
        }

        rust_entries.erase(it);
    }

    for (const auto &entry : rust_entries) {
        missing_in_legacy.push_back(entry.second.original);
    }

    if (missing_in_rust.empty() && missing_in_legacy.empty() && size_mismatches.empty() && type_mismatches.empty()) {
        return;
    }

#ifdef C4ENGINE
    if (!missing_in_rust.empty()) {
        LogNTr(spdlog::level::warn,
               "Rust group validation {}: entries missing from Rust view: {}",
               full_name.getData(), SummariseList(missing_in_rust));
    }
    if (!missing_in_legacy.empty()) {
        LogNTr(spdlog::level::warn,
               "Rust group validation {}: additional entries reported by Rust: {}",
               full_name.getData(), SummariseList(missing_in_legacy));
    }
    if (!size_mismatches.empty()) {
        LogNTr(spdlog::level::warn,
               "Rust group validation {}: size mismatch for entries: {}",
               full_name.getData(), SummariseList(size_mismatches));
    }
    if (!type_mismatches.empty()) {
        LogNTr(spdlog::level::warn,
               "Rust group validation {}: entry type mismatch for: {}",
               full_name.getData(), SummariseList(type_mismatches));
    }
#endif
}

} // namespace RustGroupBridge

#endif // USE_RUST_GROUP_VALIDATION
