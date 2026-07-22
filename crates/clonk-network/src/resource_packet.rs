//! Binary codecs for LegacyClonk's resource-transfer packet family.
//!
//! This module mirrors the `StdCompilerBin{Read,Write}` layouts in the C++
//! oracle. In particular, fixed-width integers use native byte order and
//! `StdIntPackAdapt` uses LegacyClonk's signed, non-zigzag packed encoding.

use std::fmt;

use clonk_engine::{LegacyCString, NetworkResourceCore};

/// `C4PacketType::PID_NetResDis` (`src/C4PacketBase.h:131-136`).
pub const PID_NET_RES_DISCOVER: u8 = 0x30;
/// `C4PacketType::PID_NetResStat` (`src/C4PacketBase.h:131-136`).
pub const PID_NET_RES_STATUS: u8 = 0x31;
/// `C4PacketType::PID_NetResDerive` (`src/C4PacketBase.h:131-136`).
pub const PID_NET_RES_DERIVE: u8 = 0x32;
/// `C4PacketType::PID_NetResReq` (`src/C4PacketBase.h:131-136`).
pub const PID_NET_RES_REQUEST: u8 = 0x33;
/// `C4PacketType::PID_NetResData` (`src/C4PacketBase.h:131-136`).
pub const PID_NET_RES_DATA: u8 = 0x34;
/// Size of `C4PacketResDiscover::iDisIDs` (`src/C4Network2Res.h:420`).
pub const DISCOVER_RESOURCE_ID_CAPACITY: usize = 16;
/// Maximum produced through stock C++ `AddDisID`; its `count + 1 >= 16`
/// check leaves one array slot unused (`src/C4Network2IO.cpp:1745-1750`).
pub const MAX_STOCK_DISCOVER_RESOURCE_IDS: usize = DISCOVER_RESOURCE_ID_CAPACITY - 1;
/// Maximum data bytes emitted by `C4Network2ResChunk::Set`. The wire `StdBuf`
/// itself can represent larger payloads (`src/C4Network2Res.h:27`;
/// `src/C4Network2Res.cpp:1230-1256`).
pub const MAX_STOCK_RESOURCE_DATA_BYTES: usize = 100 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiscoverPacket {
    pub resource_ids: Vec<i32>,
}

impl ResourceDiscoverPacket {
    /// Mirrors `C4PacketResDiscover::AddDisID`, including its 15-of-16 limit.
    pub fn add_resource_id(&mut self, resource_id: i32) -> bool {
        if self.resource_ids.len() >= MAX_STOCK_DISCOVER_RESOURCE_IDS {
            return false;
        }
        self.resource_ids.push(resource_id);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceChunkRange {
    pub start: i32,
    pub length: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceChunkAvailability {
    pub chunk_count: i32,
    pub ranges: Vec<ResourceChunkRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceStatusPacket {
    pub resource_id: i32,
    pub chunks: ResourceChunkAvailability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRequestPacket {
    pub resource_id: i32,
    pub chunk: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDataPacket {
    pub resource_id: i32,
    pub chunk: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcePacket {
    Discover(ResourceDiscoverPacket),
    Status(ResourceStatusPacket),
    Derive(NetworkResourceCore),
    Request(ResourceRequestPacket),
    Data(ResourceDataPacket),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcePacketCodecError {
    UnexpectedEof,
    PackedIntegerOverflow,
    UnsupportedPacket(u8),
    DiscoverCountOutOfRange(i32),
    NegativeChunkRangeCount(i32),
    TooManyChunkRanges(usize),
    ZeroResourceChunkSize,
    InvalidResourceSha,
    ResourceDataTooLarge(usize),
}

impl fmt::Display for ResourcePacketCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => formatter.write_str("resource packet is truncated"),
            Self::PackedIntegerOverflow => {
                formatter.write_str("resource packet packed integer exceeds 32 bits")
            }
            Self::UnsupportedPacket(packet_id) => {
                write!(formatter, "unsupported resource packet 0x{packet_id:02x}")
            }
            Self::DiscoverCountOutOfRange(count) => write!(
                formatter,
                "resource discover count {count} is outside the C++ 16-ID array"
            ),
            Self::NegativeChunkRangeCount(count) => {
                write!(
                    formatter,
                    "resource status has negative range count {count}"
                )
            }
            Self::TooManyChunkRanges(count) => {
                write!(
                    formatter,
                    "resource status has {count} ranges; count exceeds int32"
                )
            }
            Self::ZeroResourceChunkSize => {
                formatter.write_str("loadable resource core has zero chunk size")
            }
            Self::InvalidResourceSha => {
                formatter.write_str("resource SHA has an invalid hexadecimal byte")
            }
            Self::ResourceDataTooLarge(size) => {
                write!(formatter, "resource data size {size} exceeds uint32")
            }
        }
    }
}

impl std::error::Error for ResourcePacketCodecError {}

pub fn decode_resource_packet(data: &[u8]) -> Result<ResourcePacket, ResourcePacketCodecError> {
    let (&packet_id, payload) = data
        .split_first()
        .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
    match packet_id {
        PID_NET_RES_DISCOVER => {
            decode_resource_discover_payload(payload).map(ResourcePacket::Discover)
        }
        PID_NET_RES_STATUS => decode_resource_status_payload(payload).map(ResourcePacket::Status),
        PID_NET_RES_DERIVE => decode_resource_core_payload(payload).map(ResourcePacket::Derive),
        PID_NET_RES_REQUEST => {
            decode_resource_request_payload(payload).map(ResourcePacket::Request)
        }
        PID_NET_RES_DATA => decode_resource_data_payload(payload).map(ResourcePacket::Data),
        packet_id => Err(ResourcePacketCodecError::UnsupportedPacket(packet_id)),
    }
}

pub fn encode_resource_packet(
    packet: &ResourcePacket,
) -> Result<Vec<u8>, ResourcePacketCodecError> {
    let mut data = Vec::new();
    match packet {
        ResourcePacket::Discover(packet) => {
            data.push(PID_NET_RES_DISCOVER);
            data.extend(encode_resource_discover_payload(packet)?);
        }
        ResourcePacket::Status(packet) => {
            data.push(PID_NET_RES_STATUS);
            data.extend(encode_resource_status_payload(packet)?);
        }
        ResourcePacket::Derive(core) => {
            data.push(PID_NET_RES_DERIVE);
            data.extend(encode_resource_core_payload(core)?);
        }
        ResourcePacket::Request(packet) => {
            data.push(PID_NET_RES_REQUEST);
            data.extend(encode_resource_request_payload(*packet));
        }
        ResourcePacket::Data(packet) => {
            data.push(PID_NET_RES_DATA);
            data.extend(encode_resource_data_payload(packet)?);
        }
    }
    Ok(data)
}

pub fn decode_resource_data_payload(
    payload: &[u8],
) -> Result<ResourceDataPacket, ResourcePacketCodecError> {
    let mut reader = Reader::new(payload);
    let resource_id = reader.read_i32()?;
    let chunk = reader.read_u32()?;
    let size = reader.read_packed_u32()?;
    let data = reader.read_bytes(size as usize)?.to_vec();
    Ok(ResourceDataPacket {
        resource_id,
        chunk,
        data,
    })
}

pub fn encode_resource_data_payload(
    packet: &ResourceDataPacket,
) -> Result<Vec<u8>, ResourcePacketCodecError> {
    let size = u32::try_from(packet.data.len())
        .map_err(|_| ResourcePacketCodecError::ResourceDataTooLarge(packet.data.len()))?;
    let mut payload = Vec::with_capacity(8 + 5 + packet.data.len());
    payload.extend_from_slice(&packet.resource_id.to_ne_bytes());
    payload.extend_from_slice(&packet.chunk.to_ne_bytes());
    encode_packed_u32(size, &mut payload);
    payload.extend_from_slice(&packet.data);
    Ok(payload)
}

pub fn decode_resource_request_payload(
    payload: &[u8],
) -> Result<ResourceRequestPacket, ResourcePacketCodecError> {
    let mut reader = Reader::new(payload);
    Ok(ResourceRequestPacket {
        resource_id: reader.read_i32()?,
        chunk: reader.read_packed_i32()?,
    })
}

pub fn encode_resource_request_payload(packet: ResourceRequestPacket) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&packet.resource_id.to_ne_bytes());
    encode_packed_i32(packet.chunk, &mut payload);
    payload
}

/// Decodes the `C4Network2ResCore` body used by `PID_NetResDerive` and other
/// network packets (`src/C4Network2Res.cpp:114-143`).
pub fn decode_resource_core_payload(
    payload: &[u8],
) -> Result<NetworkResourceCore, ResourcePacketCodecError> {
    Reader::new(payload).read_resource_core()
}

/// Encodes a `C4Network2ResCore` with the exact binary compiler layout.
pub fn encode_resource_core_payload(
    core: &NetworkResourceCore,
) -> Result<Vec<u8>, ResourcePacketCodecError> {
    if core.loadable && core.chunk_size == 0 {
        return Err(ResourcePacketCodecError::ZeroResourceChunkSize);
    }

    let mut payload = Vec::new();
    payload.push(core.resource_type);
    payload.extend_from_slice(&core.id.to_ne_bytes());
    payload.extend_from_slice(&core.derived_id.to_ne_bytes());
    payload.push(u8::from(core.loadable));
    if core.loadable {
        payload.extend_from_slice(&core.file_size.to_ne_bytes());
        payload.extend_from_slice(&core.file_crc.to_ne_bytes());
        payload.extend_from_slice(&core.chunk_size.to_ne_bytes());
    }
    payload.extend_from_slice(&core.contents_crc.to_ne_bytes());
    match core.file_sha {
        Some(digest) => {
            // fHasFileSHA is uint8_t and NamingCountAdapt therefore occupies
            // one byte in the binary compiler.
            payload.push(1);
            // StdHexAdapt is non-verbose in binary mode, but does not return
            // after writing Raw: it writes both the raw digest and 20 strings.
            payload.extend_from_slice(&digest);
            digest.iter().for_each(|byte| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                payload.push(HEX[usize::from(byte >> 4)]);
                payload.push(HEX[usize::from(byte & 0x0f)]);
                payload.push(0);
            });
        }
        None => payload.push(0),
    }
    append_network_filename(&mut payload, &core.filename);
    append_network_filename(&mut payload, &core.author);
    Ok(payload)
}

pub fn decode_resource_status_payload(
    payload: &[u8],
) -> Result<ResourceStatusPacket, ResourcePacketCodecError> {
    let mut reader = Reader::new(payload);
    let resource_id = reader.read_i32()?;
    let chunk_count = reader.read_packed_i32()?;
    let range_count = reader.read_packed_i32()?;
    if range_count < 0 {
        return Err(ResourcePacketCodecError::NegativeChunkRangeCount(
            range_count,
        ));
    }
    let ranges = (0..range_count)
        .map(|_| {
            Ok(ResourceChunkRange {
                start: reader.read_packed_i32()?,
                length: reader.read_packed_i32()?,
            })
        })
        .collect::<Result<Vec<_>, ResourcePacketCodecError>>()?;
    Ok(ResourceStatusPacket {
        resource_id,
        chunks: ResourceChunkAvailability {
            chunk_count,
            ranges,
        },
    })
}

pub fn encode_resource_status_payload(
    packet: &ResourceStatusPacket,
) -> Result<Vec<u8>, ResourcePacketCodecError> {
    let range_count = i32::try_from(packet.chunks.ranges.len())
        .map_err(|_| ResourcePacketCodecError::TooManyChunkRanges(packet.chunks.ranges.len()))?;
    let mut payload = Vec::new();
    payload.extend_from_slice(&packet.resource_id.to_ne_bytes());
    encode_packed_i32(packet.chunks.chunk_count, &mut payload);
    encode_packed_i32(range_count, &mut payload);
    packet.chunks.ranges.iter().for_each(|range| {
        encode_packed_i32(range.start, &mut payload);
        encode_packed_i32(range.length, &mut payload);
    });
    Ok(payload)
}

pub fn decode_resource_discover_payload(
    payload: &[u8],
) -> Result<ResourceDiscoverPacket, ResourcePacketCodecError> {
    let mut reader = Reader::new(payload);
    let count = reader.read_packed_i32()?;
    if !(0..=DISCOVER_RESOURCE_ID_CAPACITY as i32).contains(&count) {
        return Err(ResourcePacketCodecError::DiscoverCountOutOfRange(count));
    }
    let resource_ids = (0..count)
        .map(|_| reader.read_i32())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ResourceDiscoverPacket { resource_ids })
}

pub fn encode_resource_discover_payload(
    packet: &ResourceDiscoverPacket,
) -> Result<Vec<u8>, ResourcePacketCodecError> {
    let count = i32::try_from(packet.resource_ids.len())
        .map_err(|_| ResourcePacketCodecError::DiscoverCountOutOfRange(i32::MAX))?;
    if packet.resource_ids.len() > DISCOVER_RESOURCE_ID_CAPACITY {
        return Err(ResourcePacketCodecError::DiscoverCountOutOfRange(count));
    }
    let mut payload = Vec::with_capacity(1 + packet.resource_ids.len() * 4);
    encode_packed_i32(count, &mut payload);
    packet
        .resource_ids
        .iter()
        .for_each(|resource_id| payload.extend_from_slice(&resource_id.to_ne_bytes()));
    Ok(payload)
}

struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, ResourcePacketCodecError> {
        let byte = self
            .data
            .get(self.offset)
            .copied()
            .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
        self.offset += 1;
        Ok(byte)
    }

    fn read_i32(&mut self) -> Result<i32, ResourcePacketCodecError> {
        let bytes: [u8; 4] = self.read_array()?;
        Ok(i32::from_ne_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, ResourcePacketCodecError> {
        let bytes: [u8; 4] = self.read_array()?;
        Ok(u32::from_ne_bytes(bytes))
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], ResourcePacketCodecError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_c_string(&mut self) -> Result<LegacyCString, ResourcePacketCodecError> {
        let tail = self
            .data
            .get(self.offset..)
            .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
        let length = tail
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
        let bytes = self.read_bytes(length)?.to_vec();
        self.read_u8()?;
        LegacyCString::from_bytes(bytes).ok_or(ResourcePacketCodecError::UnexpectedEof)
    }

    fn read_network_filename(&mut self) -> Result<LegacyCString, ResourcePacketCodecError> {
        let filename = self.read_c_string()?;
        #[cfg(windows)]
        {
            Ok(filename)
        }
        #[cfg(not(windows))]
        {
            let native = filename
                .as_bytes()
                .iter()
                .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
                .collect();
            LegacyCString::from_bytes(native).ok_or(ResourcePacketCodecError::UnexpectedEof)
        }
    }

    fn read_resource_core(&mut self) -> Result<NetworkResourceCore, ResourcePacketCodecError> {
        let resource_type = self.read_u8()?;
        let id = self.read_i32()?;
        let derived_id = self.read_i32()?;
        let loadable = self.read_u8()? != 0;
        let defaults = NetworkResourceCore::default();
        let (file_size, file_crc, chunk_size) = if loadable {
            (self.read_u32()?, self.read_u32()?, self.read_u32()?)
        } else {
            (defaults.file_size, defaults.file_crc, defaults.chunk_size)
        };
        if loadable && chunk_size == 0 {
            return Err(ResourcePacketCodecError::ZeroResourceChunkSize);
        }
        let contents_crc = self.read_u32()?;
        let file_sha = if self.read_u8()? == 0 {
            None
        } else {
            Some(self.read_resource_sha()?)
        };
        let filename = self.read_network_filename()?;
        let author = self.read_network_filename()?;
        Ok(NetworkResourceCore {
            resource_type,
            id,
            derived_id,
            loadable,
            file_size,
            file_crc,
            chunk_size,
            contents_crc,
            file_sha,
            filename,
            author,
        })
    }

    fn read_resource_sha(&mut self) -> Result<[u8; 20], ResourcePacketCodecError> {
        // The following textual byte strings overwrite these raw bytes in the
        // C++ reader (src/StdAdaptors.h:1029-1048).
        self.read_bytes(20)?;
        let mut digest = [0; 20];
        for byte in &mut digest {
            let encoded = self.read_c_string()?;
            let [high, low] = encoded.as_bytes() else {
                return Err(ResourcePacketCodecError::InvalidResourceSha);
            };
            *byte = (decode_hex_nibble(*high)? << 4) | decode_hex_nibble(*low)?;
        }
        Ok(digest)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], ResourcePacketCodecError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .ok_or(ResourcePacketCodecError::UnexpectedEof)?;
        self.offset = end;
        bytes
            .try_into()
            .map_err(|_| ResourcePacketCodecError::UnexpectedEof)
    }

    fn read_packed_i32(&mut self) -> Result<i32, ResourcePacketCodecError> {
        let first = self.read_u8()?;
        let mut current = first;
        let mut signed = sign_extend_seven(current);
        let mut value = signed;
        let mut length = 1usize;
        let mut shift = 7u32;

        while signed as u8 != current {
            if length == 5 {
                return Err(ResourcePacketCodecError::PackedIntegerOverflow);
            }
            current = self.read_u8()?;
            signed = sign_extend_seven(current);
            let lower_mask = (1i64 << shift) - 1;
            value = (((i64::from(signed)) << shift) | (i64::from(value) & lower_mask)) as i32;
            length += 1;
            shift += 7;
        }

        Ok(value)
    }

    fn read_packed_u32(&mut self) -> Result<u32, ResourcePacketCodecError> {
        let first = self.read_u8()?;
        let mut current = first;
        let mut chunk = u32::from(current & 0x7f);
        let mut value = chunk;
        let mut length = 1usize;
        let mut shift = 7u32;

        while chunk as u8 != current {
            if length == 5 {
                return Err(ResourcePacketCodecError::PackedIntegerOverflow);
            }
            current = self.read_u8()?;
            chunk = u32::from(current & 0x7f);
            if shift == 28 && chunk > 0x0f {
                return Err(ResourcePacketCodecError::PackedIntegerOverflow);
            }
            value |= chunk << shift;
            length += 1;
            shift += 7;
        }

        Ok(value)
    }
}

fn sign_extend_seven(byte: u8) -> i32 {
    (i32::from(byte) << 25) >> 25
}

fn encode_packed_i32(mut value: i32, output: &mut Vec<u8>) {
    loop {
        let chunk = (value << 25) >> 25;
        if chunk == value {
            output.push(chunk as u8);
            break;
        }
        output.push((chunk ^ 0x80) as u8);
        value >>= 7;
    }
}

fn encode_packed_u32(mut value: u32, output: &mut Vec<u8>) {
    loop {
        let chunk = value & 0x7f;
        if chunk == value {
            output.push(chunk as u8);
            break;
        }
        output.push((chunk as u8) ^ 0x80);
        value >>= 7;
    }
}

fn append_network_filename(output: &mut Vec<u8>, filename: &LegacyCString) {
    #[cfg(windows)]
    output.extend_from_slice(filename.as_bytes());
    #[cfg(not(windows))]
    output.extend(
        filename
            .as_bytes()
            .iter()
            .map(|byte| if *byte == b'/' { b'\\' } else { *byte }),
    );
    output.push(0);
}

fn decode_hex_nibble(byte: u8) -> Result<u8, ResourcePacketCodecError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ResourcePacketCodecError::InvalidResourceSha),
    }
}
