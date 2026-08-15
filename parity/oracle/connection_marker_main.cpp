// Bounded C++ wire proof for the optional Rust port marker.
//
// The build script mechanically extracts the production method bodies from
// the pinned LegacyClonk source before compiling this file. The declarations
// below therefore come from C4Network2IO.h/C4Client.h, and the decode enters
// the production C4PacketBase::unpack path rather than the Rust decoder.

#include <cassert>
#include <cstdint>
#include <cstring>

#include "C4Network2IO.h"
#include "C4Version.h"

// C4Network2Address methods used by C4NetIOPacket's value-owned address.
#include "conn_address_methods.inc"

// C4ClientCore's constructor/destructor/CompileFunc from C4Client.cpp.
namespace C4InVal
{
// The wire fixture contains valid names. Keep validation out of this bounded
// decoder proof while retaining the production ValidatedStdStrBuf type.
bool ValidateString(StdStrBuf &, ValidationOption)
{
    return false;
}
} // namespace C4InVal
#include "conn_client_methods.inc"

// C4PacketConn/C4PacketConnRe constructors and CompileFunc bodies from
// C4Network2IO.cpp.
#include "conn_packet_methods.inc"

// StdCompilerBinRead and NameGuard methods from StdCompiler.cpp, plus the
// string-buffer helper from StdBuf.cpp.
#include "conn_compiler_methods.inc"
#include "conn_strbuf_methods.inc"

// The packet framing entry point and packet-buffer constructors from the
// production C4Packet2.cpp/C4NetIO.cpp sources.
#include "conn_packet_base_methods.inc"

int main()
{
    // C4PacketConnRe: OK=true, empty Message, WrongPassword=false, followed
    // by the four bytes Rust reserves as its positive port-peer marker.
    const unsigned char replyBytes[] = {
        PID_ConnRe, 0x01, 0x00, 0x00, 'L', 'C', 'P', 0x01,
    };
    C4NetIOPacket replyPacket(replyBytes, sizeof(replyBytes), false);
    C4PacketConnRe reply;
    char replyStatus = 0;
    reply.unpack(replyPacket, &replyStatus);
    if (replyStatus != PID_ConnRe || !reply.isOK() || *reply.getMsg() != '\0' ||
        reply.isPasswordWrong())
        return 1;

    // C4PacketConn: C4ClientCore fields, Version=362, empty Password,
    // ConnID=0x01020304, and the same trailing marker. This is the Rust wire
    // payload used by transport::sends_cpp_connection_request_frame.
    const unsigned char requestBytes[] = {
        PID_Conn,
        0xff, 0xff, 0xff, 0xff, 0x00, 0x01, 'A', 'l', 'i', 'c', 'e', 0x00,
        'A', 'l', 'i', 0x00, 0x00, 0x6a, 0x02, 0x00, 0x84, 0x86, 0x88, 0x08,
        'L', 'C', 'P', 0x01,
    };
    C4NetIOPacket requestPacket(requestBytes, sizeof(requestBytes), false);
    C4PacketConn request;
    char requestStatus = 0;
    request.unpack(requestPacket, &requestStatus);
    if (requestStatus != PID_Conn)
        return 2;

    const auto &core = request.getCCore();
    if (core.getID() != -1 || std::strcmp(core.getName(), "Alice") != 0 ||
        std::strcmp(core.getNick(), "Ali") != 0 || request.getVer() != 362 ||
        request.getConnID() != 0x01020304u || *request.getPassword() != '\0')
        return 3;

    return 0;
}
