//! C++-faithful packet-forwarding wire model.

use std::fmt;

/// `C4PacketType::PID_FwdReq` (`src/C4PacketBase.h:95`).
pub const PID_FORWARD_REQUEST: u8 = 0x04;
/// `C4PacketType::PID_Fwd` (`src/C4PacketBase.h:96`).
pub const PID_FORWARD: u8 = 0x05;
/// Capacity of `C4PacketFwd::iClients` (`src/C4Network2IO.h:41,397`).
pub const MAX_FORWARD_CLIENTS: usize = 256;

/// Shared body of `PID_FwdReq` and `PID_Fwd`.
///
/// `nested_packet` is the complete forwarded `C4NetIOPacket`, starting with
/// its packet ID. It deliberately remains opaque at this transport boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardPacket {
    pub negative_list: bool,
    pub clients: Vec<i32>,
    pub nested_packet: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardPacketCodecError {
    UnexpectedEof,
    PackedIntegerOverflow,
    ClientCountOutOfRange(i32),
    TooManyClients(usize),
    NestedPacketTooLarge(usize),
}

impl fmt::Display for ForwardPacketCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => formatter.write_str("forward packet is truncated"),
            Self::PackedIntegerOverflow => {
                formatter.write_str("forward packet packed integer exceeds 32 bits")
            }
            Self::ClientCountOutOfRange(count) => write!(
                formatter,
                "forward packet client count {count} is outside the C++ 256-client array"
            ),
            Self::TooManyClients(count) => write!(
                formatter,
                "forward packet has {count} clients; count exceeds the C++ 256-client array"
            ),
            Self::NestedPacketTooLarge(size) => {
                write!(
                    formatter,
                    "forwarded nested packet size {size} exceeds uint32"
                )
            }
        }
    }
}

impl std::error::Error for ForwardPacketCodecError {}

/// Decodes the body after either forwarding PID.
pub fn decode_forward_packet_payload(
    payload: &[u8],
) -> Result<ForwardPacket, ForwardPacketCodecError> {
    let mut reader = Reader::new(payload);
    let negative_list = reader.read_u8()? != 0;
    let count = reader.read_packed_i32()?;
    if !(0..=MAX_FORWARD_CLIENTS as i32).contains(&count) {
        return Err(ForwardPacketCodecError::ClientCountOutOfRange(count));
    }
    let clients = (0..count)
        .map(|_| reader.read_packed_i32())
        .collect::<Result<Vec<_>, _>>()?;
    let nested_size = reader.read_packed_u32()? as usize;
    let nested_packet = reader.read_bytes(nested_size)?.to_vec();
    Ok(ForwardPacket {
        negative_list,
        clients,
        nested_packet,
    })
}

/// Encodes the body shared by both forwarding PIDs.
pub fn encode_forward_packet_payload(
    packet: &ForwardPacket,
) -> Result<Vec<u8>, ForwardPacketCodecError> {
    if packet.clients.len() > MAX_FORWARD_CLIENTS {
        return Err(ForwardPacketCodecError::TooManyClients(
            packet.clients.len(),
        ));
    }
    let nested_size = u32::try_from(packet.nested_packet.len())
        .map_err(|_| ForwardPacketCodecError::NestedPacketTooLarge(packet.nested_packet.len()))?;
    let mut payload = Vec::new();
    payload.push(u8::from(packet.negative_list));
    encode_packed_i32(packet.clients.len() as i32, &mut payload);
    packet
        .clients
        .iter()
        .for_each(|client_id| encode_packed_i32(*client_id, &mut payload));
    encode_packed_u32(nested_size, &mut payload);
    payload.extend_from_slice(&packet.nested_packet);
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

    fn read_u8(&mut self) -> Result<u8, ForwardPacketCodecError> {
        let value = self
            .data
            .get(self.offset)
            .copied()
            .ok_or(ForwardPacketCodecError::UnexpectedEof)?;
        self.offset += 1;
        Ok(value)
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], ForwardPacketCodecError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ForwardPacketCodecError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .ok_or(ForwardPacketCodecError::UnexpectedEof)?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_packed_i32(&mut self) -> Result<i32, ForwardPacketCodecError> {
        let first = self.read_u8()?;
        let mut current = first;
        let mut signed = sign_extend_seven(current);
        let mut value = signed;
        let mut length = 1usize;
        let mut shift = 7u32;

        while signed as u8 != current {
            if length == 5 {
                return Err(ForwardPacketCodecError::PackedIntegerOverflow);
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

    fn read_packed_u32(&mut self) -> Result<u32, ForwardPacketCodecError> {
        let first = self.read_u8()?;
        let mut current = first;
        let mut chunk = u32::from(current & 0x7f);
        let mut value = chunk;
        let mut length = 1usize;
        let mut shift = 7u32;

        while chunk as u8 != current {
            if length == 5 {
                return Err(ForwardPacketCodecError::PackedIntegerOverflow);
            }
            current = self.read_u8()?;
            chunk = u32::from(current & 0x7f);
            if shift == 28 && chunk > 0x0f {
                return Err(ForwardPacketCodecError::PackedIntegerOverflow);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_list_preserves_ordered_signed_client_ids_and_nested_pid() {
        // C4PacketFwd compiles each ordered int32 Client through
        // StdIntPackAdapt, then StdBuf keeps the complete nested packet
        // including its ID (src/C4Network2IO.cpp:1675-1681;
        // src/StdBuf.cpp:86-100; src/StdAdaptors.h:749-810).
        let packet = ForwardPacket {
            negative_list: false,
            clients: vec![1, -1, 128, -129],
            nested_packet: vec![0x12, 0xaa, 0xbb],
        };
        let expected = [
            0x00, 0x04, 0x01, 0xff, 0x80, 0x01, 0x7f, 0xfe, 0x03, 0x12, 0xaa, 0xbb,
        ];

        assert_eq!(encode_forward_packet_payload(&packet).unwrap(), expected);
        assert_eq!(decode_forward_packet_payload(&expected).unwrap(), packet);
    }

    #[test]
    fn decoder_rejects_counts_outside_the_cpp_client_array() {
        // C4PacketFwd owns exactly C4NetMaxClients (256) client slots
        // (src/C4Network2IO.h:41,390-399). Reject a hostile count before
        // reading or allocating its claimed list.
        assert_eq!(
            decode_forward_packet_payload(&[0x00, 0xff]),
            Err(ForwardPacketCodecError::ClientCountOutOfRange(-1))
        );
        assert_eq!(
            decode_forward_packet_payload(&[0x00, 0x81, 0x02]),
            Err(ForwardPacketCodecError::ClientCountOutOfRange(257))
        );
    }

    #[test]
    fn decoder_reports_truncated_and_overwide_forward_fields() {
        // StdIntPackAdapt and StdBuf each fail at the first missing field;
        // the Rust boundary additionally caps packed integers at their C++
        // 32-bit width (src/StdAdaptors.h:749-810; src/StdBuf.cpp:86-100).
        assert_eq!(
            decode_forward_packet_payload(&[]),
            Err(ForwardPacketCodecError::UnexpectedEof)
        );
        assert_eq!(
            decode_forward_packet_payload(&[0x00, 0x00]),
            Err(ForwardPacketCodecError::UnexpectedEof)
        );
        assert_eq!(
            decode_forward_packet_payload(&[0x00, 0x00, 0x80, 0x80, 0x80, 0x80, 0x80,]),
            Err(ForwardPacketCodecError::PackedIntegerOverflow)
        );
        assert_eq!(
            decode_forward_packet_payload(&[0x00, 0x00, 0x04, 0x40, 0x01, 0x00]),
            Err(ForwardPacketCodecError::UnexpectedEof)
        );
    }

    #[test]
    fn encoder_rejects_more_clients_than_cpp_can_store() {
        // SetData cannot enlarge C4PacketFwd's fixed 256-client array, and
        // AddClient silently stops at that capacity (src/C4Network2IO.h:
        // 390-399; src/C4Network2IO.cpp:1668-1673).
        let packet = ForwardPacket {
            negative_list: false,
            clients: vec![0; MAX_FORWARD_CLIENTS + 1],
            nested_packet: vec![0xff],
        };

        assert_eq!(
            encode_forward_packet_payload(&packet),
            Err(ForwardPacketCodecError::TooManyClients(
                MAX_FORWARD_CLIENTS + 1
            ))
        );
    }
}
