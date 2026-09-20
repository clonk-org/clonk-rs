// Compiled with the pinned oracle's headers, flags and real object files.
// Only this test executable redirects the Rust open path for fault fixtures.
#include "C4Group.h"
#include "C4Application.h"
#include "C4Log.h"
#include "lc_group_ffi.h"

#include <filesystem>
#include <iostream>
#include <spdlog/sinks/ostream_sink.h>

static const char *rust_view = nullptr;
extern "C" GroupHandle *probe_lc_group_open(const char *path)
{
    return lc_group_open(rust_view ? rust_view : path);
}

static void Require(bool value, const std::string &message)
{
    if (!value) throw std::runtime_error(message);
}

static void CompareABI(C4Group &legacy, const char *path)
{
    auto *handle = lc_group_open(path);
    Require(handle, "Rust could not open group");
    size_t count = 0;
    auto *entries = lc_group_entries(handle, &count);
    Require(entries, "Rust could not enumerate group");
    Require(count == static_cast<size_t>(legacy.EntryCount()), "entry count differs");
    std::vector<std::string> native_order;
    char name[_MAX_FNAME + 1];
    legacy.ResetSearch();
    for (bool found = legacy.FindEntry("*", name); found; found = legacy.FindNextEntry("*", name))
        native_order.emplace_back(name);
    Require(native_order.size() == count, "native enumeration count differs");
    // C4Group.cpp:1980-2020 supplies native sizes and child flags.
    for (size_t index = 0; index < count; ++index)
    {
        const auto &entry = entries[index];
        Require(native_order[index] == entry.path, "entry order differs");
        size_t size = 0;
        bool child = false;
        legacy.ResetSearch();
        Require(legacy.FindEntry(entry.path, nullptr, &size, &child), "entry missing from C++");
        const bool directory = child || std::filesystem::is_directory(std::filesystem::path(path) / entry.path);
        Require(entry.is_directory == directory, "entry type differs");
        Require(lc_group_exists(handle, entry.path), "exists returned false");
        if (directory) continue;
        Require(entry.size == size, "file size differs");
        StdBuf bytes;
        Require(legacy.LoadEntry(entry.path, bytes), "C++ could not read entry");
        size_t length = 0;
        auto *buffer = lc_group_read_file(handle, entry.path, &length);
        Require(buffer && length == bytes.getSize(), "read length differs");
        Require(!length || std::memcmp(buffer, bytes.getData(), length) == 0, "file bytes differ");
        lc_group_buffer_free(buffer, length);
    }
    lc_group_entries_free(entries, count);
    Require(!lc_group_exists(handle, "DoesNotExist"), "missing entry exists");
    size_t missing_length = 999;
    auto *missing = lc_group_read_file(handle, "DoesNotExist", &missing_length);
    Require(!missing && missing_length == 0, "missing read did not return null/zero");
    lc_group_buffer_free(missing, missing_length);
    char *root = lc_group_root(handle);
    Require(root && std::string(root) == path, "root string differs");
    lc_group_string_free(root);
    char *maker = lc_group_maker(handle);
    // C4Group.cpp:2278-2281 returns the packed header's maker field.
    if (legacy.IsPacked()) Require(maker && std::string(maker) == legacy.GetMaker(), "maker differs");
    else Require(!maker, "folder unexpectedly has a packed maker");
    lc_group_string_free(maker);
    lc_group_free(handle);
    lc_group_free(nullptr);
    lc_group_entries_free(nullptr, 0);
    lc_group_buffer_free(nullptr, 0);
    lc_group_string_free(nullptr);
}

int main(int argc, char **argv)
{
    try
    {
        unsetenv("LC_RUST_GROUP_FAULT");
        unsetenv("LC_RUST_GROUP_DEEP");
        auto logger = Application.LogSystem.GetLogger();
        logger->set_level(spdlog::level::warn);
        logger->sinks().clear();
        auto sink = std::make_shared<spdlog::sinks::ostream_sink_mt>(std::cout);
        sink->set_pattern("%v");
        logger->sinks().push_back(sink);
        if (argc == 4 && std::string(argv[1]) == "pack")
        {
            // Use the oracle's writer so packed fixtures are independent of Rust.
            C4Group_SetMaker("group differential fixture");
            C4Group_SetSortList(C4CFN_FLS);
            Require(C4Group_PackDirectoryTo(argv[2], argv[3]), "C++ pack failed");
        }
        else if (argc == 4 && std::string(argv[1]) == "compare")
        {
            rust_view = std::string(argv[2]) == argv[3] ? nullptr : argv[3];
            C4Group legacy;
            Require(legacy.Open(argv[2]), "C++ could not open group");
            // Fault cases intentionally compare different real groups. ABI
            // equality checks apply only when both parsers receive one fixture.
            if (std::string(argv[2]) == argv[3]) CompareABI(legacy, argv[2]);
        }
        else throw std::runtime_error("usage: probe pack <folder> <packed> | compare <C++ path> <Rust path>");
    }
    catch (const std::exception &error)
    {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
