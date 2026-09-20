#pragma once
#include <iostream>
#include <string>
#include <sstream>
namespace spdlog::level { enum level_enum { info, warn }; }
inline void ProbeFormat(std::string &) {}
template<typename First, typename... Rest>
void ProbeFormat(std::string &message, const First &first, const Rest &...rest) {
    const auto position = message.find("{}");
    std::ostringstream value;
    value << first;
    if (position != std::string::npos) message.replace(position, 2, value.str());
    ProbeFormat(message, rest...);
}
template<typename... Args>
void LogNTr(spdlog::level::level_enum level, std::string message, const Args &...args) {
    if (level != spdlog::level::warn) return;
    ProbeFormat(message, args...);
    std::cout << message << '\n';
}
