#!/usr/bin/env bash
# Verify that the pinned LegacyClonk C++ connection packets accept the Rust
# marker as trailing bytes. This is a bounded wire proof, not a full native
# engine/network build: it uses the real packet declarations and production
# C4PacketBase::unpack entrypoint, with source methods mechanically extracted
# from the pinned oracle snapshot.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
default_oracle_revision="7d43b47b7d789b533f32d005e64596e0a07019cd"
oracle_repo="${LEGACYCLONK_ORACLE_ROOT:-$repo}"
oracle_revision="${LEGACYCLONK_ORACLE_REVISION:-$default_oracle_revision}"
gen="$here/.connection-marker-gen"

if ! oracle_commit="$(git -C "$oracle_repo" rev-parse --verify "$oracle_revision^{commit}")"; then
    echo "C++ oracle revision $oracle_revision not found in $oracle_repo" >&2
    exit 1
fi

mkdir -p "$gen"
oracle_snapshot="$gen/oracle-src-$oracle_commit"
if [[ ! -f "$oracle_snapshot/.complete" ]]; then
    mkdir -p "$oracle_snapshot"
    git -C "$oracle_repo" archive "$oracle_commit" src | tar -x -C "$oracle_snapshot"
    touch "$oracle_snapshot/.complete"
fi
src="$oracle_snapshot/src"

# These hashes pin every production source file whose declarations, packet
# entrypoint, compiler, string, or endpoint methods are used below. If the
# oracle moves, this proof fails closed until its source review is refreshed.
check_hash() {
    local path="$1"
    local expected="$2"
    local actual
    actual="$(git -C "$oracle_repo" show "$oracle_commit:$path" | sha256sum | awk '{print $1}')"
    if [[ "$actual" != "$expected" ]]; then
        echo "oracle source hash mismatch: $path" >&2
        echo "  expected $expected" >&2
        echo "  actual   $actual" >&2
        exit 1
    fi
}

check_hash src/C4Network2IO.cpp b8b8b0fca3f40243710ee3c9882497aaed7d3a9097f2bb5e3a5703cf17269d84
check_hash src/C4Network2IO.h 87e9f3f477dfa73a2afd4b74b83984ecb143189ed10a589a0422e83c9c3f1dd8
check_hash src/C4Client.cpp 53005ba202a545ed891b317a7989050a5c10ae6c8644ebd197887ae081667cec
check_hash src/C4Client.h 09f6d8864a422cfc1fea4024cded2e2e97c5b06508128c4a192cd065bce5585a
check_hash src/C4Packet2.cpp 9b02cdbc4e1a325cf3be7efeb2100e1f428081c2959b5a850fe638bc3a34766d
check_hash src/C4PacketBase.h d7946335d58509a1dbfc1330bbc712b411d9e9df50eb9770f489183598f88d08
check_hash src/C4Version.h c94a3b78b5e4ffb8a69bfac863d32ed3472ac651a77ee779159c376f6ffecded
check_hash src/C4Network2Address.cpp 74f734f1c7c6af366efcf13172e75fd28d07768e20f74c0233eceab9d3c9f93f
check_hash src/C4Network2Address.h 94abd8a0ee35814ba6e70bb81a26ed35e61538e573ecfddef64461022a2423c2
check_hash src/C4NetIO.cpp 2bfb6dda222663680ed4ad7f1284a97f030c93e9c69dc627eed5996fe8f1449e
check_hash src/C4NetIO.h 419a2e491241c08d3fe6523a0c853b79e9d4972fffe2f5375b85a8fce0920d8e
check_hash src/StdCompiler.cpp 47795e276b7e1326bc449d18b346c222536d56b86d36d5f9c0442f042708134a
check_hash src/StdCompiler.h 04d936f6ab382ef05356278fb8d0336f87dc8da1054f95a67183b16a48384411
check_hash src/StdBuf.cpp babc9c0b4a4be79ac4391597bb6962fd8f29aab580a3f0971b42a1d9308d8d1f
check_hash src/StdBuf.h 46c6e0602d5c87a477695cfabc8ae7874a9c0758915f240ff71bedfb27679300

# Extract a complete C++ function, stopping on balanced braces. Constructors
# such as `C4PacketConn() : iVer(...) {}` finish on the same line, so a
# line-oriented `^}$` extractor would silently produce an incomplete proof.
extract_function() {
    local input="$1"
    local pattern="$2"
    local output="$3"
    awk -v re="$pattern" '
        function opens(line,    n) { n = line; gsub(/[^{}]/, "", n); return gsub(/{/, "", n) }
        function closes(line,    n) { n = line; gsub(/[^{}]/, "", n); return gsub(/}/, "", n) }
        !started && index($0, re) { started = 1 }
        started {
            print
            if (opens($0) > 0) saw_open = 1
            if (saw_open) depth += opens($0) - closes($0)
            if (saw_open && depth == 0) { found = 1; exit }
        }
        END { if (!found) exit 1 }
    ' "$input" > "$output"
}

extract_lines() {
    local input="$1"
    local first="$2"
    local last="$3"
    local output="$4"
    awk -v first="$first" -v last="$last" '
        !started && index($0, first) { started = 1 }
        started { print }
        started && index($0, last) { found = 1; exit }
        END { if (!found) exit 1 }
    ' "$input" > "$output"
}

extract_reader_template() {
    local input="$1"
    local output="$2"
    awk '
        function opens(line,    n) { n = line; gsub(/[^{}]/, "", n); return gsub(/{/, "", n) }
        function closes(line,    n) { n = line; gsub(/[^{}]/, "", n); return gsub(/}/, "", n) }
        !started && /^template <class T>$/ { candidate = 1; next }
        candidate && index($0, "inline void StdCompilerBinRead::ReadValue") {
            print "template <class T>"
            started = 1
        }
        started {
            print
            if (opens($0) > 0) saw_open = 1
            if (saw_open) depth += opens($0) - closes($0)
            if (saw_open && depth == 0) { found = 1; exit }
        }
        END { if (!found) exit 1 }
    ' "$input" > "$output"
}

# C4Network2Address.cpp: only the endpoint value operations reached by the
# production C4NetIOPacket constructors are needed by the framed fixture.
: > "$gen/conn_address_methods.inc"
for spec in \
    "$src/C4Network2Address.cpp|void C4Network2HostAddress::Clear" \
    "$src/C4Network2Address.cpp|void C4Network2HostAddress::SetHost(const C4Network2HostAddress" \
    "$src/C4Network2Address.cpp|void C4Network2HostAddress::SetHost(const sockaddr" \
    "$src/C4Network2Address.cpp|void C4Network2EndpointAddress::Clear" \
    "$src/C4Network2Address.cpp|void C4Network2EndpointAddress::SetAddress(const C4Network2EndpointAddress" \
    "$src/C4Network2Address.cpp|void C4Network2EndpointAddress::SetPort" \
    "$src/C4Network2Address.cpp|std::uint16_t C4Network2EndpointAddress::GetPort"; do
    input="${spec%%|*}"
    pattern="${spec#*|}"
    tmp="$gen/conn-method.tmp"
    extract_function "$input" "$pattern" "$tmp"
    cat "$tmp" >> "$gen/conn_address_methods.inc"
done

# C4Client.cpp: C4ClientCore's real serialization fields and initialization.
: > "$gen/conn_client_methods.inc"
for pattern in \
    'C4ClientCore::C4ClientCore' \
    'C4ClientCore::~C4ClientCore' \
    'void C4ClientCore::CompileFunc'; do
    if [[ "$pattern" == *'~C4ClientCore'* ]]; then
        awk -v re="$pattern" 'index($0, re) { print; found = 1; exit } END { if (!found) exit 1 }' \
            "$src/C4Client.cpp" >> "$gen/conn_client_methods.inc"
    else
        tmp="$gen/conn-method.tmp"
        extract_function "$src/C4Client.cpp" "$pattern" "$tmp"
        cat "$tmp" >> "$gen/conn_client_methods.inc"
    fi
done

# C4Network2IO.cpp: actual Conn/ConnRe constructors and field compilers.
: > "$gen/conn_packet_methods.inc"
for pattern in \
    'C4PacketConn::C4PacketConn()' \
    'C4PacketConn::C4PacketConn(const C4ClientCore' \
    'void C4PacketConn::CompileFunc' \
    'C4PacketConnRe::C4PacketConnRe()' \
    'C4PacketConnRe::C4PacketConnRe(bool' \
    'void C4PacketConnRe::CompileFunc'; do
    tmp="$gen/conn-method.tmp"
    extract_function "$src/C4Network2IO.cpp" "$pattern" "$tmp"
    cat "$tmp" >> "$gen/conn_packet_methods.inc"
done

# StdCompiler.cpp: the production binary reader and guard methods. Keep the
# consecutive scalar methods as one exact source range.
: > "$gen/conn_compiler_methods.inc"
for pattern in \
    'StdCompiler::NameGuard::NameGuard(NameGuard &&' \
    'StdCompiler::NameGuard &StdCompiler::NameGuard::operator=' \
    'StdCompiler::NameGuard::~NameGuard' \
    'void StdCompiler::NameGuard::End' \
    'void StdCompiler::NameGuard::Abort' \
    'void StdCompiler::NameGuard::Disarm'; do
    tmp="$gen/conn-method.tmp"
    extract_function "$src/StdCompiler.cpp" "$pattern" "$tmp"
    cat "$tmp" >> "$gen/conn_compiler_methods.inc"
done
tmp="$gen/conn-method.tmp"
extract_lines "$src/StdCompiler.cpp" 'void StdCompilerBinRead::QWord(int64_t' 'void StdCompilerBinRead::Character(char' "$tmp"
cat "$tmp" >> "$gen/conn_compiler_methods.inc"
for pattern in \
    'void StdCompilerBinRead::String(char *' \
    'void StdCompilerBinRead::String(std::string' \
    'void StdCompilerBinRead::Raw' \
    'std::string StdCompilerBinRead::getPosition' \
    'void StdCompilerBinRead::Begin'; do
    tmp="$gen/conn-method.tmp"
    extract_function "$src/StdCompiler.cpp" "$pattern" "$tmp"
    cat "$tmp" >> "$gen/conn_compiler_methods.inc"
done
tmp="$gen/conn-method.tmp"
extract_reader_template "$src/StdCompiler.cpp" "$tmp"
cat "$tmp" >> "$gen/conn_compiler_methods.inc"

# StdBuf.cpp: the real string-buffer compiler used by both packet classes.
extract_function "$src/StdBuf.cpp" 'void StdStrBuf::CompileFunc' "$gen/conn_strbuf_methods.inc"

# C4Packet2.cpp and C4NetIO.cpp: the actual packet unpack entrypoint and the
# packet-buffer constructors used to provide a status/PID frame.
: > "$gen/conn_packet_base_methods.inc"
for spec in \
    "$src/C4Packet2.cpp|void C4PacketBase::unpack" \
    "$src/C4NetIO.cpp|C4NetIOPacket::C4NetIOPacket()" \
    "$src/C4NetIO.cpp|C4NetIOPacket::C4NetIOPacket(const void" \
    "$src/C4NetIO.cpp|C4NetIOPacket::C4NetIOPacket(const StdBuf" \
    "$src/C4NetIO.cpp|C4NetIOPacket::~C4NetIOPacket" \
    "$src/C4NetIO.cpp|void C4NetIOPacket::Clear"; do
    input="${spec%%|*}"
    pattern="${spec#*|}"
    tmp="$gen/conn-method.tmp"
    extract_function "$input" "$pattern" "$tmp"
    cat "$tmp" >> "$gen/conn_packet_base_methods.inc"
done

cxx="${CXX:-clang++}"
"$cxx" -std=c++23 -O0 -DC4ENGINE \
    -Wno-deprecated-copy-with-user-provided-copy \
    -Wno-missing-designated-field-initializers \
    -Wno-ignored-qualifiers \
    -Wno-parentheses \
    -I"$here/connection_marker_support" -I"$gen" -I"$src" \
    "$here/connection_marker_main.cpp" -lfmt \
    -o "$gen/connection_marker_oracle"

"$gen/connection_marker_oracle"
echo "C++ connection marker proof passed for $oracle_commit"
