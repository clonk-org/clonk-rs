use flate2::{Compress, Compression, FlushCompress, Status};
use thiserror::Error;

/// `C4NetStreamingMinBlockSize`: live record uploads wait for at least this
/// much compressed data.
pub const LEAGUE_STREAM_MIN_BLOCK_SIZE: usize = 10 * 1024;
/// `C4NetStreamingMaxBlockSize`: initial live compression capacity.
pub const LEAGUE_STREAM_MAX_BLOCK_SIZE: usize = 20 * 1024;
/// `C4NetStreamingInterval`, in whole seconds.
pub const LEAGUE_STREAM_INTERVAL_SECONDS: i64 = 30;
/// `C4Record::StreamFile`'s record-stream-only file chunk.
pub const LEAGUE_STREAM_FILE_CHUNK_TYPE: u8 = 0x30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueRecordUpload {
    pub endpoint: String,
    pub position: u32,
    pub end: bool,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeagueRecordStreamPhase {
    Running,
    Finishing,
    Stopped,
}

#[derive(Debug, Error)]
pub enum LeagueRecordStreamError {
    #[error("league record stream compression failed: {0}")]
    Compression(#[from] flate2::CompressError),
    #[error("league record stream cannot accept bytes after finishing")]
    AppendAfterFinish,
    #[error("league record stream finish made no compression progress")]
    FinishStalled,
    #[error("league record stream buffer cannot grow further")]
    BufferOverflow,
    #[error("streamed record file has {0} bytes, exceeding the C++ uint32 length field")]
    FileTooLarge(usize),
    #[error("league record stream has no upload awaiting acknowledgement")]
    NoUploadInFlight,
}

/// Encode one `C4Record::StreamFile` chunk.
///
/// The binary payload is the NUL-terminated legacy filename followed by a
/// 7-bit packed `uint32_t` byte length and the raw file image. These chunks
/// carry the initial save and later resource files before the live CtrlRec
/// chunks in the same uncompressed record stream.
pub fn encode_league_stream_file_chunk(
    filename: &lc_engine::LegacyCString,
    file: &[u8],
) -> Result<Vec<u8>, LeagueRecordStreamError> {
    let file_len =
        u32::try_from(file.len()).map_err(|_| LeagueRecordStreamError::FileTooLarge(file.len()))?;
    let mut encoded = Vec::with_capacity(
        2_usize
            .saturating_add(filename.as_bytes().len())
            .saturating_add(1)
            .saturating_add(5)
            .saturating_add(file.len()),
    );
    encoded.extend_from_slice(&[0, LEAGUE_STREAM_FILE_CHUNK_TYPE]);
    encoded.extend_from_slice(filename.as_bytes());
    encoded.push(0);
    encode_packed_u32(file_len, &mut encoded);
    encoded.extend_from_slice(file);
    Ok(encoded)
}

fn encode_packed_u32(mut value: u32, encoded: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        encoded.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// C++'s persistent zlib record-upload state machine.
///
/// One compressor spans every HTTP body. Live output is capped at 20 KiB,
/// while `finish` grows the buffer until `Z_FINISH` reaches `Z_STREAM_END`.
/// Upload acknowledgement, rather than dispatch, advances the compressed
/// byte position and releases the sent prefix.
pub struct LeagueRecordStream {
    endpoint: String,
    compressor: Compress,
    raw_pending: Vec<u8>,
    compressed_pending: Vec<u8>,
    output_capacity: usize,
    position: u32,
    in_flight: Option<usize>,
    last_attempt: i64,
    phase: LeagueRecordStreamPhase,
    compression_finished: bool,
}

impl LeagueRecordStream {
    pub fn new(endpoint: impl Into<String>, now: i64) -> Self {
        Self {
            endpoint: endpoint.into(),
            compressor: Compress::new(Compression::best(), true),
            raw_pending: Vec::new(),
            compressed_pending: Vec::new(),
            output_capacity: LEAGUE_STREAM_MAX_BLOCK_SIZE,
            position: 0,
            in_flight: None,
            last_attempt: now,
            phase: LeagueRecordStreamPhase::Running,
            compression_finished: false,
        }
    }

    pub fn append(&mut self, bytes: &[u8]) -> Result<(), LeagueRecordStreamError> {
        if self.phase != LeagueRecordStreamPhase::Running {
            return Err(LeagueRecordStreamError::AppendAfterFinish);
        }
        self.raw_pending.extend_from_slice(bytes);
        Ok(())
    }

    /// Mirror the one-second `StreamIn(false); StreamOut()` pump.
    pub fn pump(
        &mut self,
        now: i64,
    ) -> Result<Option<LeagueRecordUpload>, LeagueRecordStreamError> {
        if self.phase == LeagueRecordStreamPhase::Stopped {
            return Ok(None);
        }
        if self.phase == LeagueRecordStreamPhase::Running {
            self.compress_live_once()?;
        }
        if self.in_flight.is_some() {
            return Ok(None);
        }

        let end = self.phase == LeagueRecordStreamPhase::Finishing;
        if !end {
            if self.compressed_pending.len() < LEAGUE_STREAM_MIN_BLOCK_SIZE {
                return Ok(None);
            }
            // Native blocks at exactly last+30 and permits the next second.
            if self.last_attempt != 0
                && self
                    .last_attempt
                    .saturating_add(LEAGUE_STREAM_INTERVAL_SECONDS)
                    >= now
            {
                return Ok(None);
            }
        } else if self.compressed_pending.is_empty() {
            if self.compression_finished {
                self.phase = LeagueRecordStreamPhase::Stopped;
            }
            return Ok(None);
        }

        let amount = self.compressed_pending.len();
        self.in_flight = Some(amount);
        self.last_attempt = now;
        Ok(Some(LeagueRecordUpload {
            endpoint: format!("{}pos={}&end={end}", self.endpoint, self.position),
            position: self.position,
            end,
            body: self.compressed_pending.clone(),
        }))
    }

    /// Apply the HTTP result. Failure retains the complete prefix for retry.
    pub fn acknowledge_upload(&mut self, success: bool) -> Result<(), LeagueRecordStreamError> {
        let amount = self
            .in_flight
            .take()
            .ok_or(LeagueRecordStreamError::NoUploadInFlight)?;
        if !success {
            return Ok(());
        }

        self.compressed_pending.drain(..amount);
        self.position = self.position.wrapping_add(amount as u32);
        if self.phase == LeagueRecordStreamPhase::Running {
            self.compress_live_once()?;
        } else if self.compression_finished && self.compressed_pending.is_empty() {
            self.phase = LeagueRecordStreamPhase::Stopped;
        }
        Ok(())
    }

    /// Consume all remaining raw bytes using `Z_FINISH`. Final uploads bypass
    /// both the live minimum size and the 30-second cadence.
    pub fn finish(&mut self) -> Result<(), LeagueRecordStreamError> {
        match self.phase {
            LeagueRecordStreamPhase::Stopped => return Ok(()),
            LeagueRecordStreamPhase::Finishing if self.compression_finished => return Ok(()),
            LeagueRecordStreamPhase::Finishing => {}
            LeagueRecordStreamPhase::Running => {
                self.phase = LeagueRecordStreamPhase::Finishing;
            }
        }

        loop {
            if self.compressed_pending.len() == self.output_capacity {
                self.grow_output()?;
            }
            let before_in = self.compressor.total_in();
            let before_out = self.compressor.total_out();
            let status = self.compress_once(FlushCompress::Finish)?;
            if status == Status::StreamEnd {
                self.compression_finished = true;
                return Ok(());
            }
            if status != Status::Ok
                || (self.compressor.total_in() == before_in
                    && self.compressor.total_out() == before_out)
            {
                return Err(LeagueRecordStreamError::FinishStalled);
            }
            // C++ doubles StreamingBuf after every non-terminal finish call.
            self.grow_output()?;
        }
    }

    pub fn is_streaming(&self) -> bool {
        self.phase != LeagueRecordStreamPhase::Stopped
    }

    pub fn position(&self) -> u32 {
        self.position
    }

    /// Uncompressed bytes accepted by the persistent zlib stream. This is
    /// C4Record's `GetStreamingPos()` diagnostic.
    pub fn input_position(&self) -> u64 {
        self.compressor.total_in()
    }

    pub fn pending_compressed_len(&self) -> usize {
        self.compressed_pending.len()
    }

    pub fn pending_raw_len(&self) -> usize {
        self.raw_pending.len()
    }

    fn compress_live_once(&mut self) -> Result<(), LeagueRecordStreamError> {
        if self.raw_pending.is_empty() || self.compressed_pending.len() == self.output_capacity {
            return Ok(());
        }
        let _ = self.compress_once(FlushCompress::None)?;
        Ok(())
    }

    fn compress_once(&mut self, flush: FlushCompress) -> Result<Status, LeagueRecordStreamError> {
        let available = self
            .output_capacity
            .saturating_sub(self.compressed_pending.len());
        if available == 0 {
            return Ok(Status::BufError);
        }
        let mut output = vec![0_u8; available];
        let before_in = self.compressor.total_in();
        let before_out = self.compressor.total_out();
        let status = self
            .compressor
            .compress(&self.raw_pending, &mut output, flush)?;
        let consumed = usize::try_from(self.compressor.total_in() - before_in)
            .unwrap_or(self.raw_pending.len())
            .min(self.raw_pending.len());
        let produced = usize::try_from(self.compressor.total_out() - before_out)
            .unwrap_or(output.len())
            .min(output.len());
        if consumed != 0 {
            self.raw_pending.drain(..consumed);
        }
        self.compressed_pending
            .extend_from_slice(&output[..produced]);
        Ok(status)
    }

    fn grow_output(&mut self) -> Result<(), LeagueRecordStreamError> {
        self.output_capacity = self
            .output_capacity
            .checked_mul(2)
            .ok_or(LeagueRecordStreamError::BufferOverflow)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::ZlibDecoder;

    use super::*;

    fn incompressible_bytes(len: usize) -> Vec<u8> {
        let mut value = 0x1234_5678_u32;
        (0..len)
            .map(|_| {
                value ^= value << 13;
                value ^= value >> 17;
                value ^= value << 5;
                value as u8
            })
            .collect()
    }

    fn decode_zlib(bytes: &[u8]) -> Vec<u8> {
        let mut decoded = Vec::new();
        ZlibDecoder::new(bytes)
            .read_to_end(&mut decoded)
            .expect("decode streamed zlib bytes");
        decoded
    }

    #[test]
    fn live_upload_uses_cpp_compressed_threshold_capacity_and_strict_cadence() {
        let source = incompressible_bytes(40 * 1024);
        let mut stream = LeagueRecordStream::new("http://stream.invalid/upload?", 100);
        stream.append(&source).unwrap();

        assert!(stream.pump(130).unwrap().is_none());
        assert_eq!(
            stream.pending_compressed_len(),
            LEAGUE_STREAM_MAX_BLOCK_SIZE
        );
        assert!(stream.pending_raw_len() > 0);

        let upload = stream.pump(131).unwrap().expect("first live upload");
        assert_eq!(
            upload.endpoint,
            "http://stream.invalid/upload?pos=0&end=false"
        );
        assert_eq!(upload.position, 0);
        assert!(!upload.end);
        assert_eq!(upload.body.len(), LEAGUE_STREAM_MAX_BLOCK_SIZE);
        stream.acknowledge_upload(true).unwrap();
        assert_eq!(stream.position(), LEAGUE_STREAM_MAX_BLOCK_SIZE as u32);

        assert!(stream.pump(161).unwrap().is_none());
        let second = stream.pump(162).unwrap().expect("second live upload");
        assert_eq!(
            second.endpoint,
            format!(
                "http://stream.invalid/upload?pos={}&end=false",
                LEAGUE_STREAM_MAX_BLOCK_SIZE
            )
        );
    }

    #[test]
    fn failed_live_upload_retries_the_same_bytes_and_position() {
        let mut stream = LeagueRecordStream::new("stream?", 10);
        stream.append(&incompressible_bytes(40 * 1024)).unwrap();
        let first = stream.pump(41).unwrap().expect("first upload");
        stream.acknowledge_upload(false).unwrap();

        assert!(stream.pump(71).unwrap().is_none());
        let retry = stream.pump(72).unwrap().expect("retry upload");
        assert_eq!(retry, first);
        assert_eq!(stream.position(), 0);
    }

    #[test]
    fn finish_bypasses_live_gates_and_stops_only_after_terminal_ack() {
        let source = b"small final record";
        let mut stream = LeagueRecordStream::new("stream?", 500);
        stream.append(source).unwrap();
        stream.finish().unwrap();

        let upload = stream.pump(500).unwrap().expect("terminal upload");
        assert!(upload.end);
        assert_eq!(upload.endpoint, "stream?pos=0&end=true");
        assert!(upload.body.len() < LEAGUE_STREAM_MIN_BLOCK_SIZE);
        assert!(stream.is_streaming());
        assert_eq!(decode_zlib(&upload.body), source);

        stream.acknowledge_upload(true).unwrap();
        assert!(!stream.is_streaming());
        assert!(stream.pump(500).unwrap().is_none());
    }

    #[test]
    fn finish_behind_an_inflight_prefix_preserves_one_continuous_zlib_stream() {
        let first_source = incompressible_bytes(40 * 1024);
        let second_source = incompressible_bytes(17 * 1024);
        let mut stream = LeagueRecordStream::new("stream?", 1_000);
        stream.append(&first_source).unwrap();
        let first = stream.pump(1_031).unwrap().expect("live prefix");

        stream.append(&second_source).unwrap();
        assert!(stream.pump(1_032).unwrap().is_none());
        stream.finish().unwrap();
        stream.acknowledge_upload(true).unwrap();

        let final_upload = stream.pump(1_032).unwrap().expect("final suffix");
        assert!(final_upload.end);
        assert_eq!(final_upload.position, first.body.len() as u32);
        let mut compressed = first.body;
        compressed.extend_from_slice(&final_upload.body);
        let mut expected = first_source;
        expected.extend_from_slice(&second_source);
        assert_eq!(decode_zlib(&compressed), expected);

        stream.acknowledge_upload(true).unwrap();
        assert!(!stream.is_streaming());
    }

    #[test]
    fn failed_live_prefix_is_retried_with_final_suffix_at_the_original_position() {
        let first_source = incompressible_bytes(40 * 1024);
        let final_source = incompressible_bytes(7 * 1024);
        let mut stream = LeagueRecordStream::new("stream?", 1_000);
        stream.append(&first_source).unwrap();
        let live = stream.pump(1_031).unwrap().expect("live prefix");

        stream.append(&final_source).unwrap();
        stream.finish().unwrap();
        stream.acknowledge_upload(false).unwrap();

        let terminal = stream.pump(1_031).unwrap().expect("terminal retry");
        assert!(terminal.end);
        assert_eq!(terminal.position, 0);
        assert!(terminal.body.starts_with(&live.body));
        let mut expected = first_source;
        expected.extend_from_slice(&final_source);
        assert_eq!(decode_zlib(&terminal.body), expected);
    }

    #[test]
    fn live_upload_requires_exact_compressed_minimum() {
        let mut stream = LeagueRecordStream::new("stream?", 10);
        stream.compressed_pending = vec![0; LEAGUE_STREAM_MIN_BLOCK_SIZE - 1];
        assert!(stream.pump(41).unwrap().is_none());

        stream.compressed_pending.push(0);
        let upload = stream.pump(41).unwrap().expect("threshold upload");
        assert_eq!(upload.body.len(), LEAGUE_STREAM_MIN_BLOCK_SIZE);
    }

    #[test]
    fn empty_finish_emits_a_terminal_zlib_stream() {
        let mut stream = LeagueRecordStream::new("stream?", 10);
        stream.finish().unwrap();

        let upload = stream.pump(10).unwrap().expect("empty terminal upload");
        assert!(upload.end);
        assert_eq!(decode_zlib(&upload.body), b"");
        stream.acknowledge_upload(true).unwrap();
        assert!(!stream.is_streaming());
    }

    #[test]
    fn failed_terminal_upload_retries_immediately_without_mutation() {
        let mut stream = LeagueRecordStream::new("stream?", 10);
        stream.append(b"final record").unwrap();
        stream.finish().unwrap();

        let first = stream.pump(10).unwrap().expect("terminal upload");
        stream.acknowledge_upload(false).unwrap();
        let retry = stream.pump(10).unwrap().expect("immediate terminal retry");
        assert_eq!(retry, first);
    }

    #[test]
    fn append_after_finish_is_rejected() {
        let mut stream = LeagueRecordStream::new("stream?", 10);
        stream.finish().unwrap();
        assert!(matches!(
            stream.append(b"late"),
            Err(LeagueRecordStreamError::AppendAfterFinish)
        ));
    }

    #[test]
    fn streamed_file_chunk_matches_cpp_binary_compiler_layout() {
        let filename = lc_engine::LegacyCString::from_bytes(b"Record.c4s".to_vec()).unwrap();
        let file = vec![0x5a; 130];
        let encoded = encode_league_stream_file_chunk(&filename, &file).unwrap();

        assert_eq!(&encoded[..14], b"\0\x30Record.c4s\0\x82");
        assert_eq!(encoded[14], 0x01);
        assert_eq!(&encoded[15..], file);
    }
}
