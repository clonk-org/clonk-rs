use crate::resource_packet::{
    decode_resource_core_payload, decode_resource_packet, encode_resource_core_payload,
    encode_resource_packet, ResourceChunkAvailability, ResourceChunkRange, ResourceDataPacket,
    ResourceDiscoverPacket, ResourcePacket, ResourcePacketCodecError, ResourceRequestPacket,
    ResourceStatusPacket, DISCOVER_RESOURCE_ID_CAPACITY, MAX_STOCK_DISCOVER_RESOURCE_IDS,
    MAX_STOCK_RESOURCE_DATA_BYTES,
};
use clonk_engine::{LegacyCString, NetworkResourceCore};

#[test]
fn cpp_resource_discover_vector_uses_packed_count_and_native_ids() {
    // C4PacketResDiscover::CompileFunc writes a packed int32 count followed by
    // native int32 resource IDs (src/C4Network2IO.cpp:1753-1757).
    let packet = ResourcePacket::Discover(ResourceDiscoverPacket {
        resource_ids: vec![0x0102_0304, -1, 128],
    });
    let expected = [
        0x30, 0x03, 0x04, 0x03, 0x02, 0x01, 0xff, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00,
    ];

    assert_eq!(encode_resource_packet(&packet).unwrap(), expected);
    assert_eq!(decode_resource_packet(&expected).unwrap(), packet);
}

#[test]
fn cpp_discover_builder_stops_at_fifteen_but_wire_accepts_sixteen() {
    // The field is a 16-element array, while AddDisID rejects the sixteenth
    // insertion due to `count + 1 >= 16` (src/C4Network2Res.h:420;
    // src/C4Network2IO.cpp:1745-1750). Binary decode can safely fill all 16.
    assert_eq!(DISCOVER_RESOURCE_ID_CAPACITY, 16);
    assert_eq!(MAX_STOCK_DISCOVER_RESOURCE_IDS, 15);
    let mut stock = ResourceDiscoverPacket {
        resource_ids: Vec::new(),
    };
    for id in 0..15 {
        assert!(stock.add_resource_id(id));
    }
    assert!(!stock.add_resource_id(15));

    let packet = ResourcePacket::Discover(ResourceDiscoverPacket {
        resource_ids: (0..16).collect(),
    });
    let encoded = encode_resource_packet(&packet).unwrap();
    assert_eq!(decode_resource_packet(&encoded).unwrap(), packet);

    let over_capacity = ResourcePacket::Discover(ResourceDiscoverPacket {
        resource_ids: (0..17).collect(),
    });
    assert!(encode_resource_packet(&over_capacity).is_err());
}

#[test]
fn cpp_resource_data_vector_is_native_header_then_stdbuf() {
    // C4Network2ResChunk writes native ResID/Chunk fields, and StdBuf writes a
    // packed uint32 size followed by raw bytes (src/C4Network2Res.cpp:1321-1328;
    // src/StdBuf.cpp:86-100).
    let packet = ResourcePacket::Data(ResourceDataPacket {
        resource_id: 0x0102_0304,
        chunk: 0xa0b0_c0d0,
        data: vec![0xde, 0xad, 0xbe],
    });
    let expected = [
        0x34, 0x04, 0x03, 0x02, 0x01, 0xd0, 0xc0, 0xb0, 0xa0, 0x03, 0xde, 0xad, 0xbe,
    ];

    assert_eq!(encode_resource_packet(&packet).unwrap(), expected);
    assert_eq!(decode_resource_packet(&expected).unwrap(), packet);
}

#[test]
fn cpp_resource_request_vector_mixes_native_id_and_packed_chunk() {
    // C4PacketResRequest::CompileFunc writes ResID as native int32 and Chunk
    // through StdIntPackAdapt<int32_t> (src/C4Network2IO.cpp:1764-1768).
    let packet = ResourcePacket::Request(ResourceRequestPacket {
        resource_id: -2,
        chunk: 128,
    });
    let expected = [0x33, 0xfe, 0xff, 0xff, 0xff, 0x80, 0x01];

    assert_eq!(encode_resource_packet(&packet).unwrap(), expected);
    assert_eq!(decode_resource_packet(&expected).unwrap(), packet);
}

#[test]
fn cpp_resource_derive_vector_is_the_complete_resource_core() {
    // PID_NetResDerive directly compiles C4Network2ResCore
    // (src/C4Packet2.cpp:90; src/C4Network2Res.cpp:114-143). Non-loadable
    // cores omit file size/CRC/chunk size and retain constructor defaults.
    let core = NetworkResourceCore {
        resource_type: 2,
        id: -1,
        derived_id: 0x0102_0304,
        loadable: false,
        contents_crc: 0x1122_3344,
        filename: crate::c4(b"Scenario.c4s"),
        author: crate::c4(b"Alice"),
        ..NetworkResourceCore::default()
    };
    let packet = ResourcePacket::Derive(core);
    let expected = [
        0x32, 0x02, 0xff, 0xff, 0xff, 0xff, 0x04, 0x03, 0x02, 0x01, 0x00, 0x44, 0x33, 0x22, 0x11,
        0x00, b'S', b'c', b'e', b'n', b'a', b'r', b'i', b'o', b'.', b'c', b'4', b's', 0x00, b'A',
        b'l', b'i', b'c', b'e', 0x00,
    ];

    assert_eq!(encode_resource_packet(&packet).unwrap(), expected);
    assert_eq!(decode_resource_packet(&expected).unwrap(), packet);
}

#[test]
fn cpp_resource_core_sha_vector_contains_raw_and_hex_forms() {
    // StdHexAdapt first writes the raw digest in non-verbose binary mode and
    // then still writes one NUL-terminated two-digit string per byte
    // (src/StdAdaptors.h:1029-1049).
    let core = NetworkResourceCore {
        resource_type: 4,
        id: 0x0102_0304,
        derived_id: -2,
        loadable: true,
        file_size: 0x1122_3344,
        file_crc: 0x5566_7788,
        chunk_size: 102_400,
        contents_crc: 0x99aa_bbcc,
        file_sha: Some([0xab; 20]),
        filename: crate::c4(b"Defs.c4d"),
        author: LegacyCString::default(),
    };
    let packet = ResourcePacket::Derive(core);
    let expected = [
        0x32, 0x04, 0x04, 0x03, 0x02, 0x01, 0xfe, 0xff, 0xff, 0xff, 0x01, 0x44, 0x33, 0x22, 0x11,
        0x88, 0x77, 0x66, 0x55, 0x00, 0x90, 0x01, 0x00, 0xcc, 0xbb, 0xaa, 0x99, 0x01, 0xab, 0xab,
        0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
        0xab, 0xab, 0xab, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b',
        0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b',
        0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b',
        0, b'a', b'b', 0, b'a', b'b', 0, b'a', b'b', 0, b'D', b'e', b'f', b's', b'.', b'c', b'4',
        b'd', 0, 0,
    ];

    assert_eq!(encode_resource_packet(&packet).unwrap(), expected);
    assert_eq!(decode_resource_packet(&expected).unwrap(), packet);
}

#[test]
fn cpp_resource_status_vector_uses_packed_range_pairs() {
    // C4PacketResStatus writes a native ResID, then C4Network2ResChunkData
    // writes packed ChunkCnt/ChunkRangeCnt and packed Start/Length pairs
    // (src/C4Network2IO.cpp:1726-1730; src/C4Network2Res.cpp:321-350).
    let packet = ResourcePacket::Status(ResourceStatusPacket {
        resource_id: 0x0102_0304,
        chunks: ResourceChunkAvailability {
            chunk_count: 300,
            ranges: vec![
                ResourceChunkRange {
                    start: 0,
                    length: 2,
                },
                ResourceChunkRange {
                    start: 128,
                    length: 172,
                },
            ],
        },
    });
    let expected = [
        0x31, 0x04, 0x03, 0x02, 0x01, 0xac, 0x02, 0x02, 0x00, 0x02, 0x80, 0x01, 0xac, 0x01,
    ];

    assert_eq!(encode_resource_packet(&packet).unwrap(), expected);
    assert_eq!(decode_resource_packet(&expected).unwrap(), packet);
}

#[test]
fn cpp_signed_pack_extremes_match_std_int_pack_adapt() {
    // StdIntPackAdapt sign-extends each seven-bit group rather than using
    // zigzag encoding (src/StdAdaptors.h:748-809).
    let maximum = ResourcePacket::Request(ResourceRequestPacket {
        resource_id: 0,
        chunk: i32::MAX,
    });
    let maximum_bytes = [0x33, 0, 0, 0, 0, 0x7f, 0x7f, 0x7f, 0x7f, 0x07];
    assert_eq!(encode_resource_packet(&maximum).unwrap(), maximum_bytes);
    assert_eq!(decode_resource_packet(&maximum_bytes).unwrap(), maximum);

    let minimum = ResourcePacket::Request(ResourceRequestPacket {
        resource_id: 0,
        chunk: i32::MIN,
    });
    let minimum_bytes = [0x33, 0, 0, 0, 0, 0x80, 0x80, 0x80, 0x80, 0xf8];
    assert_eq!(encode_resource_packet(&minimum).unwrap(), minimum_bytes);
    assert_eq!(decode_resource_packet(&minimum_bytes).unwrap(), minimum);
}

#[test]
fn cpp_stdbuf_length_crosses_the_one_byte_boundary_at_128() {
    // StdBuf packs its uint32 size with StdIntPackAdapt
    // (src/StdBuf.cpp:86-100; src/StdAdaptors.h:741-746).
    let packet = ResourcePacket::Data(ResourceDataPacket {
        resource_id: 0,
        chunk: 0,
        data: vec![0x5a; 128],
    });
    let encoded = encode_resource_packet(&packet).unwrap();
    assert_eq!(&encoded[9..11], &[0x80, 0x01]);
    assert_eq!(decode_resource_packet(&encoded).unwrap(), packet);

    let mut truncated = encoded;
    truncated.pop();
    assert_eq!(
        decode_resource_packet(&truncated),
        Err(ResourcePacketCodecError::UnexpectedEof)
    );
}

#[test]
fn cpp_packet_unpack_tolerates_trailing_payload_bytes() {
    // CompileFromBuf does not check the binary reader position after Compile
    // (src/StdCompiler.h:221-249,382-387), so stock ignores trailing bytes.
    let bytes = [0x33, 0x04, 0x03, 0x02, 0x01, 0x07, 0xde, 0xad];
    assert_eq!(
        decode_resource_packet(&bytes).unwrap(),
        ResourcePacket::Request(ResourceRequestPacket {
            resource_id: 0x0102_0304,
            chunk: 7,
        })
    );
}

#[test]
fn cpp_sha_hex_strings_overwrite_the_raw_digest_on_decode() {
    // StdHexAdapt reads the raw bytes first, then assigns each parsed string
    // back into the same digest (src/StdAdaptors.h:1029-1048).
    let core = NetworkResourceCore {
        resource_type: 1,
        loadable: true,
        file_size: 1,
        file_crc: 2,
        chunk_size: 3,
        contents_crc: 4,
        file_sha: Some([0xab; 20]),
        ..NetworkResourceCore::default()
    };
    let mut payload = encode_resource_core_payload(&core).unwrap();
    payload[27..47].fill(0);
    assert_eq!(
        decode_resource_core_payload(&payload).unwrap().file_sha,
        Some([0xab; 20])
    );
}

#[test]
fn zero_chunk_size_is_corrupt_only_for_loadable_cores() {
    // C4Network2ResCore checks zero ChunkSize inside the Loadable branch
    // (src/C4Network2Res.cpp:129-136).
    let loadable = NetworkResourceCore {
        loadable: true,
        chunk_size: 0,
        ..NetworkResourceCore::default()
    };
    assert_eq!(
        encode_resource_core_payload(&loadable),
        Err(ResourcePacketCodecError::ZeroResourceChunkSize)
    );

    let non_loadable = NetworkResourceCore {
        chunk_size: 0,
        ..NetworkResourceCore::default()
    };
    let decoded =
        decode_resource_core_payload(&encode_resource_core_payload(&non_loadable).unwrap())
            .unwrap();
    assert_eq!(decoded.chunk_size, 102_400);
}

#[test]
fn malformed_wire_counts_are_rejected_before_allocation() {
    assert_eq!(
        decode_resource_packet(&[0x30, 0xff]),
        Err(ResourcePacketCodecError::DiscoverCountOutOfRange(-1))
    );
    assert_eq!(
        decode_resource_packet(&[0x30, 17]),
        Err(ResourcePacketCodecError::DiscoverCountOutOfRange(17))
    );
    assert_eq!(
        decode_resource_packet(&[0x31, 0, 0, 0, 0, 0, 0xff]),
        Err(ResourcePacketCodecError::NegativeChunkRangeCount(-1))
    );
}

#[test]
fn malformed_packed_integers_stop_at_the_int32_and_uint32_width() {
    let signed_overflow = [0x33, 0, 0, 0, 0, 0x80, 0x80, 0x80, 0x80, 0x80];
    assert_eq!(
        decode_resource_packet(&signed_overflow),
        Err(ResourcePacketCodecError::PackedIntegerOverflow)
    );

    let unsigned_overflow = [0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(
        decode_resource_packet(&unsigned_overflow),
        Err(ResourcePacketCodecError::PackedIntegerOverflow)
    );
}

#[test]
fn invalid_sha_hex_is_rejected_even_when_raw_sha_is_present() {
    let core = NetworkResourceCore {
        loadable: true,
        file_size: 1,
        file_crc: 2,
        chunk_size: 3,
        file_sha: Some([0xab; 20]),
        ..NetworkResourceCore::default()
    };
    let mut payload = encode_resource_core_payload(&core).unwrap();
    payload[47] = b'g';
    assert_eq!(
        decode_resource_core_payload(&payload),
        Err(ResourcePacketCodecError::InvalidResourceSha)
    );
}

#[cfg(not(windows))]
#[test]
fn network_filenames_translate_native_slashes_like_cpp() {
    // C4NetFilenameAdapt changes '/' to '\\' before binary write and reverses
    // it after read on non-Windows platforms (src/C4PacketBase.h:43-64).
    let core = NetworkResourceCore {
        filename: crate::c4(b"dir/file.c4d"),
        author: crate::c4(b"team/name"),
        ..NetworkResourceCore::default()
    };
    let payload = encode_resource_core_payload(&core).unwrap();
    assert!(payload
        .windows(b"dir\\file.c4d\0".len())
        .any(|window| window == b"dir\\file.c4d\0"));
    assert!(payload
        .windows(b"team\\name\0".len())
        .any(|window| window == b"team\\name\0"));
    assert_eq!(decode_resource_core_payload(&payload).unwrap(), core);
}

#[test]
fn stock_data_chunk_limit_is_distinct_from_the_wire_size_field() {
    // The sender caps file reads at C4NetResChunkSize, while StdBuf uses a
    // uint32 length and the decoder has no corresponding 100-KiB check.
    assert_eq!(MAX_STOCK_RESOURCE_DATA_BYTES, 102_400);
    let packet = ResourcePacket::Data(ResourceDataPacket {
        resource_id: 1,
        chunk: 0,
        data: vec![0; MAX_STOCK_RESOURCE_DATA_BYTES + 1],
    });
    let encoded = encode_resource_packet(&packet).unwrap();
    assert_eq!(decode_resource_packet(&encoded).unwrap(), packet);
}
