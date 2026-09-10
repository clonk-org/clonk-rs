/// C4RecordChunkType values (C4Record.h:58-61).
pub const RCT_CTRL: u8 = 0x00;
pub const RCT_CTRL_PKT: u8 = 0x01;
pub const RCT_FRAME: u8 = 0x02;
pub const RCT_END: u8 = 0x10;

/// Binary control record stream mirroring `C4Record::Rec`
/// (C4Record.cpp:243-264): 2-byte chunk heads `{frame_diff: u8, type: u8}`
/// (C4RecordChunkHead, C4Record.h:109-113) followed by the raw payload, with
/// empty RCT_Frame filler chunks whenever the frame difference exceeds 0xff,
/// and an RCT_End head at `frame + 37` on close (C4Record.cpp:195-197 — the
/// u8 head field truncates the sum).
#[derive(Debug, Clone, Default)]
pub struct BinaryControlRecord {
    bytes: Vec<u8>,
    last_frame: u32,
}

impl BinaryControlRecord {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rec(&mut self, frame: u32, payload: &[u8], chunk_type: u8) {
        // filler chunks (C4Record.cpp:245-247)
        while frame.saturating_sub(self.last_frame) > 0xff {
            // The difference check guarantees this addition cannot overflow.
            let filler_frame = self.last_frame + 0xff;
            self.rec(filler_frame, &[], RCT_FRAME);
        }
        // get frame difference (C4Record.cpp:248-250)
        let frame_diff = frame.saturating_sub(self.last_frame);
        self.last_frame += frame_diff;
        // head + payload (C4Record.cpp:252-255)
        self.bytes.push(frame_diff as u8);
        self.bytes.push(chunk_type);
        self.bytes.extend_from_slice(payload);
    }

    /// Write the end marker (C4Record.cpp:194-197): an RCT_End head whose u8
    /// frame field carries `frame + 37` truncated.
    pub fn finish(&mut self, frame: u32) {
        self.bytes.push(frame.wrapping_add(37) as u8);
        self.bytes.push(RCT_END);
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod binary_record_tests {
    use super::*;

    #[test]
    fn binary_record_chunk_stream_matches_cpp_format() {
        // C4Record::Rec (C4Record.cpp:243-264) + Stop end marker
        // (C4Record.cpp:194-197).
        let mut record = BinaryControlRecord::new();
        record.rec(5, b"AB", RCT_CTRL);
        record.rec(5, b"C", RCT_CTRL_PKT);
        record.rec(600, b"", RCT_FRAME);
        record.finish(600);
        let bytes = record.into_bytes();
        // chunk 1: diff 5 from frame 0, control payload
        assert_eq!(&bytes[0..4], &[5, RCT_CTRL, b'A', b'B']);
        // chunk 2: same frame → diff 0
        assert_eq!(&bytes[4..7], &[0, RCT_CTRL_PKT, b'C']);
        // frame 600 needs fillers at 5+255=260 and 260+255=515, then diff 85
        assert_eq!(&bytes[7..9], &[255, RCT_FRAME]);
        assert_eq!(&bytes[9..11], &[255, RCT_FRAME]);
        assert_eq!(&bytes[11..13], &[85, RCT_FRAME]);
        // end head: (600 + 37) & 0xff = 125
        assert_eq!(&bytes[13..15], &[125, RCT_END]);
        assert_eq!(bytes.len(), 15);
    }

    #[test]
    fn binary_record_earlier_frame_clamps_diff_to_zero() {
        // C4Record.cpp:249: iLastFrame > iFrame → diff 0 (no rewind).
        let mut record = BinaryControlRecord::new();
        record.rec(10, b"", RCT_CTRL);
        record.rec(3, b"", RCT_CTRL);
        let bytes = record.into_bytes();
        assert_eq!(&bytes[0..2], &[10, RCT_CTRL]);
        assert_eq!(&bytes[2..4], &[0, RCT_CTRL]);
    }

    #[test]
    fn binary_record_handles_uint32_end_frames_like_cpp() {
        let mut record = BinaryControlRecord::new();
        record.finish(u32::MAX);
        let bytes = record.into_bytes();
        assert_eq!(bytes, [36, RCT_END]);
    }
}
