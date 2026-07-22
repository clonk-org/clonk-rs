use std::sync::Arc;

use crate::decoder::AudioDecodeError;

const WAVE_FORMAT_PCM: u16 = 0x0001;
const WAVE_FORMAT_ADPCM: u16 = 0x0002;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const WAVE_FORMAT_ALAW: u16 = 0x0006;
const WAVE_FORMAT_MULAW: u16 = 0x0007;
const WAVE_FORMAT_IMA_ADPCM: u16 = 0x0011;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
const MAX_MIXER_CHANNELS: usize = 8;
const MAX_CONVERTIBLE_SAMPLE_RATE: u32 = 4_194_302;

const EXTENSIBLE_GUID_TAIL: [u8; 12] = [
    0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

const MS_ADPCM_COEFFICIENTS: [(i16, i16); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

const MS_ADPCM_ADAPTATION: [i32; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];

const IMA_INDEX_ADJUSTMENT: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

const IMA_STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

pub(crate) struct WavStream {
    decoder: WavDecoder,
}

enum WavDecoder {
    RawPcm(RawPcmWavStream),
    Law(LawWavStream),
    Adpcm(AdpcmWavStream),
}

impl WavStream {
    pub(crate) fn new(data: Arc<[u8]>) -> Result<Self, AudioDecodeError> {
        // Keep every WAV family on the same checked RIFF path so eager sound
        // effects and streaming music reject malformed inputs consistently.
        let decoder = parsed_wav_stream(data)?;
        Ok(Self { decoder })
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        match &self.decoder {
            WavDecoder::RawPcm(stream) => stream.sample_rate,
            WavDecoder::Law(stream) => stream.sample_rate,
            WavDecoder::Adpcm(stream) => stream.sample_rate,
        }
    }

    pub(crate) fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        match &mut self.decoder {
            WavDecoder::RawPcm(stream) => stream.next_frame(),
            WavDecoder::Law(stream) => stream.next_frame(),
            WavDecoder::Adpcm(stream) => stream.next_frame(),
        }
    }

    #[cfg(test)]
    pub(crate) fn buffered_frames(&self) -> usize {
        match &self.decoder {
            WavDecoder::RawPcm(_) | WavDecoder::Law(_) => 0,
            WavDecoder::Adpcm(stream) => stream.frames.capacity(),
        }
    }

    #[cfg(test)]
    pub(crate) fn peak_buffered_frames(&self) -> usize {
        match &self.decoder {
            WavDecoder::RawPcm(_) | WavDecoder::Law(_) => 0,
            WavDecoder::Adpcm(stream) => stream.peak_buffered_frames,
        }
    }
}

#[derive(Clone, Copy)]
enum RawPcmSampleKind {
    Int8,
    Int16,
    Int24,
    Int32,
    Float32,
}

struct RawPcmWavStream {
    data: Arc<[u8]>,
    position: usize,
    end: usize,
    sample_rate: u32,
    channels: usize,
    bytes_per_sample: usize,
    sample_kind: RawPcmSampleKind,
}

impl RawPcmWavStream {
    fn new(data: Arc<[u8]>, parsed: &ParsedWave) -> Result<Self, AudioDecodeError> {
        let (sample_kind, bytes_per_sample): (RawPcmSampleKind, usize) =
            match (parsed.effective_format, parsed.bits_per_sample) {
                (WAVE_FORMAT_PCM, 8) => (RawPcmSampleKind::Int8, 1),
                (WAVE_FORMAT_PCM, 16) => (RawPcmSampleKind::Int16, 2),
                (WAVE_FORMAT_PCM, 24) => (RawPcmSampleKind::Int24, 3),
                (WAVE_FORMAT_PCM, 32) => (RawPcmSampleKind::Int32, 4),
                (WAVE_FORMAT_IEEE_FLOAT, 32) => (RawPcmSampleKind::Float32, 4),
                _ => {
                    return Err(AudioDecodeError::InvalidData(
                        "unsupported WAV bit depth or format",
                    ));
                }
            };
        let frame_size = bytes_per_sample
            .checked_mul(parsed.channels)
            .ok_or(AudioDecodeError::InvalidData("WAV frame size overflow"))?;
        let expected_byte_rate = parsed
            .sample_rate
            .checked_mul(u32::try_from(frame_size).map_err(|_| {
                AudioDecodeError::InvalidData("WAV frame size exceeds byte-rate range")
            })?)
            .ok_or(AudioDecodeError::InvalidData("WAV byte rate overflow"))?;
        if parsed.block_align != frame_size
            || parsed.byte_rate != expected_byte_rate
            || (parsed.data_end - parsed.data_start) % frame_size != 0
        {
            return Err(AudioDecodeError::InvalidData(
                "invalid WAV block alignment or byte rate",
            ));
        }
        Ok(Self {
            data,
            position: parsed.data_start,
            end: parsed.data_end,
            sample_rate: parsed.sample_rate,
            channels: parsed.channels,
            bytes_per_sample,
            sample_kind,
        })
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        let channels = self.channels;
        read_stereo_frame(channels, || self.next_sample())
    }

    fn next_sample(&mut self) -> Result<Option<f32>, AudioDecodeError> {
        if self.position == self.end {
            return Ok(None);
        }
        let sample_end = self
            .position
            .checked_add(self.bytes_per_sample)
            .filter(|end| *end <= self.end)
            .ok_or(AudioDecodeError::InvalidData("incomplete WAV sample"))?;
        let bytes = &self.data[self.position..sample_end];
        self.position = sample_end;
        let sample = match self.sample_kind {
            RawPcmSampleKind::Int8 => (f32::from(bytes[0]) - 128.0) / 128.0,
            RawPcmSampleKind::Int16 => {
                f32::from(i16::from_le_bytes(
                    bytes.try_into().expect("two-byte sample"),
                )) / f32::from(i16::MAX)
            }
            RawPcmSampleKind::Int24 => {
                let unsigned =
                    i32::from(bytes[0]) | (i32::from(bytes[1]) << 8) | (i32::from(bytes[2]) << 16);
                let signed = if unsigned & 0x80_0000 == 0 {
                    unsigned
                } else {
                    unsigned | !0xff_ffff
                };
                signed as f32 / 8_388_607.0
            }
            RawPcmSampleKind::Int32 => {
                i32::from_le_bytes(bytes.try_into().expect("four-byte sample")) as f32
                    / i32::MAX as f32
            }
            RawPcmSampleKind::Float32 => {
                f32::from_le_bytes(bytes.try_into().expect("four-byte sample")).clamp(-1.0, 1.0)
            }
        };
        Ok(Some(sample))
    }
}

#[derive(Clone, Copy)]
enum LawKind {
    A,
    Mu,
}

struct LawWavStream {
    data: Arc<[u8]>,
    position: usize,
    end: usize,
    sample_rate: u32,
    channels: usize,
    kind: LawKind,
}

impl LawWavStream {
    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        if self.position == self.end {
            return Ok(None);
        }
        let frame_end = self
            .position
            .checked_add(self.channels)
            .filter(|end| *end <= self.end)
            .ok_or(AudioDecodeError::InvalidData("incomplete WAV x-law frame"))?;
        let position = self.position;
        let kind = self.kind;
        let frame = stereo_i16_from(self.channels, |channel| match kind {
            LawKind::A => decode_a_law(self.data[position + channel]),
            LawKind::Mu => decode_mu_law(self.data[position + channel]),
        });
        self.position = frame_end;
        Ok(Some(frame))
    }
}

#[derive(Clone)]
enum AdpcmCodec {
    Ms { coefficients: Vec<(i16, i16)> },
    Ima,
}

struct AdpcmWavStream {
    data: Arc<[u8]>,
    position: usize,
    end: usize,
    sample_rate: u32,
    channels: usize,
    block_align: usize,
    samples_per_block: usize,
    codec: AdpcmCodec,
    frames: Vec<[f32; 2]>,
    frame_position: usize,
    #[cfg(test)]
    peak_buffered_frames: usize,
}

impl AdpcmWavStream {
    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        loop {
            if let Some(frame) = self.frames.get(self.frame_position).copied() {
                self.frame_position += 1;
                return Ok(Some(frame));
            }
            if self.position == self.end {
                return Ok(None);
            }
            self.load_block()?;
        }
    }

    fn load_block(&mut self) -> Result<(), AudioDecodeError> {
        let block_end = self
            .position
            .checked_add(self.block_align)
            .unwrap_or(usize::MAX)
            .min(self.end);
        let block = &self.data[self.position..block_end];
        let mut frames = match &self.codec {
            AdpcmCodec::Ms { coefficients } => {
                decode_ms_adpcm_block(block, self.channels, coefficients)?
            }
            AdpcmCodec::Ima => decode_ima_adpcm_block(block, self.channels)?,
        };
        frames.truncate(self.samples_per_block);
        if frames.is_empty() {
            return Err(AudioDecodeError::InvalidData("empty WAV ADPCM block"));
        }
        self.position = block_end;
        self.frames = frames;
        self.frame_position = 0;
        #[cfg(test)]
        {
            self.peak_buffered_frames = self.peak_buffered_frames.max(self.frames.capacity());
        }
        Ok(())
    }
}

struct ParsedWave {
    raw_format: u16,
    effective_format: u16,
    channels: usize,
    sample_rate: u32,
    byte_rate: u32,
    block_align: usize,
    bits_per_sample: u16,
    fmt: Vec<u8>,
    data_start: usize,
    data_end: usize,
}

fn parsed_wav_stream(data: Arc<[u8]>) -> Result<WavDecoder, AudioDecodeError> {
    let parsed = parse_wave(data.as_ref())?;
    match parsed.effective_format {
        WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT => {
            let stream = RawPcmWavStream::new(data, &parsed)?;
            Ok(WavDecoder::RawPcm(stream))
        }
        WAVE_FORMAT_ALAW | WAVE_FORMAT_MULAW => {
            if parsed.bits_per_sample != 8 || parsed.block_align != parsed.channels {
                return Err(AudioDecodeError::InvalidData("invalid WAV x-law format"));
            }
            if (parsed.data_end - parsed.data_start) % parsed.channels != 0 {
                return Err(AudioDecodeError::InvalidData("incomplete WAV x-law frame"));
            }
            Ok(WavDecoder::Law(LawWavStream {
                data,
                position: parsed.data_start,
                end: parsed.data_end,
                sample_rate: parsed.sample_rate,
                channels: parsed.channels,
                kind: if parsed.effective_format == WAVE_FORMAT_ALAW {
                    LawKind::A
                } else {
                    LawKind::Mu
                },
            }))
        }
        WAVE_FORMAT_ADPCM => {
            if parsed.raw_format == WAVE_FORMAT_EXTENSIBLE {
                return Err(AudioDecodeError::InvalidData(
                    "extensible MS ADPCM WAV is unsupported",
                ));
            }
            let (samples_per_block, coefficients) = parse_ms_adpcm_format(&parsed)?;
            validate_adpcm_data(
                parsed.data_end - parsed.data_start,
                parsed.block_align,
                7 * parsed.channels,
                |_| true,
            )?;
            Ok(WavDecoder::Adpcm(AdpcmWavStream {
                data,
                position: parsed.data_start,
                end: parsed.data_end,
                sample_rate: parsed.sample_rate,
                channels: parsed.channels,
                block_align: parsed.block_align,
                samples_per_block,
                codec: AdpcmCodec::Ms { coefficients },
                frames: Vec::with_capacity(samples_per_block),
                frame_position: 0,
                #[cfg(test)]
                peak_buffered_frames: 0,
            }))
        }
        WAVE_FORMAT_IMA_ADPCM => {
            let samples_per_block = parse_ima_adpcm_format(&parsed)?;
            let header_len = 4 * parsed.channels;
            let group_len = 4 * parsed.channels;
            validate_adpcm_data(
                parsed.data_end - parsed.data_start,
                parsed.block_align,
                header_len,
                |payload| payload % group_len == 0,
            )?;
            Ok(WavDecoder::Adpcm(AdpcmWavStream {
                data,
                position: parsed.data_start,
                end: parsed.data_end,
                sample_rate: parsed.sample_rate,
                channels: parsed.channels,
                block_align: parsed.block_align,
                samples_per_block,
                codec: AdpcmCodec::Ima,
                frames: Vec::with_capacity(samples_per_block),
                frame_position: 0,
                #[cfg(test)]
                peak_buffered_frames: 0,
            }))
        }
        _ => Err(AudioDecodeError::InvalidData("unsupported WAV encoding")),
    }
}

fn parse_wave(data: &[u8]) -> Result<ParsedWave, AudioDecodeError> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(AudioDecodeError::InvalidData("invalid WAV header"));
    }
    let riff_end = usize::try_from(read_u32(data, 4)?)
        .ok()
        .and_then(|length| length.checked_add(8))
        .filter(|end| *end >= 12 && *end <= data.len())
        .ok_or(AudioDecodeError::InvalidData("invalid WAV header"))?;

    let mut position = 12;
    let mut fmt_range = None;
    let mut data_range = None;
    let mut chunks = 0_usize;
    while position < riff_end {
        chunks += 1;
        if chunks > 10_000 || riff_end - position < 8 {
            return Err(AudioDecodeError::InvalidData("invalid WAV chunk header"));
        }
        let chunk_id = &data[position..position + 4];
        let chunk_len = usize::try_from(read_u32(data, position + 4)?)
            .map_err(|_| AudioDecodeError::InvalidData("WAV chunk is too large"))?;
        let payload_start = position + 8;
        let payload_end = payload_start
            .checked_add(chunk_len)
            .filter(|end| *end <= riff_end)
            .ok_or(AudioDecodeError::InvalidData("truncated WAV chunk"))?;
        if chunk_id == b"fmt " && fmt_range.is_none() {
            if data_range.is_some() {
                return Err(AudioDecodeError::InvalidData("WAV fmt follows data"));
            }
            fmt_range = Some((payload_start, payload_end));
        } else if chunk_id == b"data" && data_range.is_none() {
            if fmt_range.is_none() {
                return Err(AudioDecodeError::InvalidData("WAV data precedes fmt"));
            }
            data_range = Some((payload_start, payload_end));
            break;
        }
        // Padding only separates chunks. Legacy Clonk WAVs commonly omit a
        // final odd-sized data chunk's pad, which is irrelevant once its
        // payload has been found.
        let padded_end = payload_end
            .checked_add(chunk_len & 1)
            .filter(|next| *next <= riff_end)
            .ok_or(AudioDecodeError::InvalidData("truncated WAV chunk padding"))?;
        position = padded_end;
    }

    let (fmt_start, fmt_end) = fmt_range.ok_or(AudioDecodeError::InvalidData("missing WAV fmt"))?;
    let (data_start, data_end) =
        data_range.ok_or(AudioDecodeError::InvalidData("missing WAV data"))?;
    let fmt = &data[fmt_start..fmt_end];
    if fmt.len() < 16 {
        return Err(AudioDecodeError::InvalidData("invalid WAV fmt size"));
    }
    let raw_format = read_u16(fmt, 0)?;
    let channels = usize::from(read_u16(fmt, 2)?);
    let sample_rate = read_u32(fmt, 4)?;
    let byte_rate = read_u32(fmt, 8)?;
    let block_align = usize::from(read_u16(fmt, 12)?);
    let bits_per_sample = read_u16(fmt, 14)?;
    if channels == 0 {
        return Err(AudioDecodeError::InvalidData("WAV channel count is zero"));
    }
    if channels > MAX_MIXER_CHANNELS {
        return Err(AudioDecodeError::InvalidData("WAV has too many channels"));
    }
    if sample_rate == 0 {
        return Err(AudioDecodeError::InvalidData("WAV sample rate is zero"));
    }
    if sample_rate > MAX_CONVERTIBLE_SAMPLE_RATE {
        return Err(AudioDecodeError::InvalidData(
            "WAV sample rate exceeds mixer conversion range",
        ));
    }
    if block_align == 0 {
        return Err(AudioDecodeError::InvalidData("WAV block alignment is zero"));
    }

    let fmt_used;
    let effective_format = if raw_format == WAVE_FORMAT_EXTENSIBLE {
        let extension_len = if fmt.len() >= 18 {
            usize::from(read_u16(fmt, 16)?)
        } else {
            0
        };
        if fmt.len() < 40
            || extension_len < 22
            || 18_usize
                .checked_add(extension_len)
                .filter(|required| *required <= fmt.len())
                .is_none()
        {
            return Err(AudioDecodeError::InvalidData(
                "invalid extensible WAV format",
            ));
        }
        fmt_used = 18 + extension_len;
        // SDL keys off the subtype GUID and ignores valid-bits/channel-mask
        // metadata, including the common 24-valid-bits-in-32-bit-container form.
        extensible_subformat(fmt).ok_or(AudioDecodeError::InvalidData(
            "unsupported extensible WAV subformat",
        ))?
    } else {
        fmt_used = if fmt.len() == 16 {
            16
        } else {
            if fmt.len() < 18 {
                return Err(AudioDecodeError::InvalidData("invalid WAV fmt extension"));
            }
            let extension_len = usize::from(read_u16(fmt, 16)?);
            18_usize
                .checked_add(extension_len)
                .filter(|required| *required <= fmt.len())
                .ok_or(AudioDecodeError::InvalidData("truncated WAV fmt extension"))?
        };
        raw_format
    };

    Ok(ParsedWave {
        raw_format,
        effective_format,
        channels,
        sample_rate,
        byte_rate,
        block_align,
        bits_per_sample,
        fmt: fmt[..fmt_used].to_vec(),
        data_start,
        data_end,
    })
}

fn parse_ms_adpcm_format(
    parsed: &ParsedWave,
) -> Result<(usize, Vec<(i16, i16)>), AudioDecodeError> {
    if parsed.bits_per_sample != 4 || !(1..=2).contains(&parsed.channels) {
        return Err(AudioDecodeError::InvalidData("invalid MS ADPCM WAV format"));
    }
    let header_len = 7 * parsed.channels;
    if parsed.block_align < header_len || parsed.fmt.len() < 22 {
        return Err(AudioDecodeError::InvalidData("invalid MS ADPCM block size"));
    }
    let coefficient_count = usize::from(read_u16(&parsed.fmt, 20)?);
    coefficient_count
        .checked_mul(4)
        .and_then(|length| length.checked_add(22))
        .filter(|end| *end <= parsed.fmt.len())
        .ok_or(AudioDecodeError::InvalidData(
            "truncated MS ADPCM coefficients",
        ))?;
    if coefficient_count < MS_ADPCM_COEFFICIENTS.len() {
        return Err(AudioDecodeError::InvalidData(
            "missing MS ADPCM coefficients",
        ));
    }
    let mut coefficients = Vec::with_capacity(coefficient_count);
    for index in 0..coefficient_count {
        let offset = 22 + index * 4;
        coefficients.push((
            read_i16(&parsed.fmt, offset)?,
            read_i16(&parsed.fmt, offset + 2)?,
        ));
    }
    if coefficients[..MS_ADPCM_COEFFICIENTS.len()] != MS_ADPCM_COEFFICIENTS {
        return Err(AudioDecodeError::InvalidData(
            "invalid MS ADPCM coefficients",
        ));
    }
    let block_capacity = 2 + (parsed.block_align - header_len) * 2 / parsed.channels;
    let declared = usize::from(read_u16(&parsed.fmt, 18)?);
    let samples_per_block = if declared == 0 {
        block_capacity
    } else {
        declared
    };
    if !(2..=block_capacity).contains(&samples_per_block) {
        return Err(AudioDecodeError::InvalidData(
            "invalid MS ADPCM samples per block",
        ));
    }
    Ok((samples_per_block, coefficients))
}

fn parse_ima_adpcm_format(parsed: &ParsedWave) -> Result<usize, AudioDecodeError> {
    if parsed.bits_per_sample != 4 {
        return Err(AudioDecodeError::InvalidData(
            "invalid IMA ADPCM WAV format",
        ));
    }
    let header_len = 4 * parsed.channels;
    let group_len = 4 * parsed.channels;
    if parsed.block_align < header_len
        || parsed.block_align % 4 != 0
        || (parsed.block_align - header_len) % group_len != 0
    {
        return Err(AudioDecodeError::InvalidData(
            "invalid IMA ADPCM block size",
        ));
    }
    let block_capacity = 1 + (parsed.block_align - header_len) * 2 / parsed.channels;
    let declared = if parsed.fmt.len() >= 20 {
        usize::from(read_u16(&parsed.fmt, 18)?)
    } else {
        0
    };
    let samples_per_block = if declared == 0 {
        block_capacity
    } else {
        declared
    };
    if !(1..=block_capacity).contains(&samples_per_block) {
        return Err(AudioDecodeError::InvalidData(
            "invalid IMA ADPCM samples per block",
        ));
    }
    Ok(samples_per_block)
}

fn validate_adpcm_data(
    data_len: usize,
    block_align: usize,
    header_len: usize,
    valid_payload: impl Fn(usize) -> bool,
) -> Result<(), AudioDecodeError> {
    if data_len % block_align != 0 {
        return Err(AudioDecodeError::InvalidData(
            "partial trailing WAV ADPCM block",
        ));
    }
    let mut position = 0;
    while position < data_len {
        let block_len = (data_len - position).min(block_align);
        if block_len < header_len || !valid_payload(block_len - header_len) {
            return Err(AudioDecodeError::InvalidData("invalid WAV ADPCM block"));
        }
        position += block_len;
    }
    Ok(())
}

fn decode_ms_adpcm_block(
    block: &[u8],
    channels: usize,
    coefficients: &[(i16, i16)],
) -> Result<Vec<[f32; 2]>, AudioDecodeError> {
    let header_len = 7 * channels;
    if block.len() < header_len {
        return Err(AudioDecodeError::InvalidData("truncated MS ADPCM block"));
    }
    let mut states = Vec::with_capacity(channels);
    for channel in 0..channels {
        let predictor = usize::from(block[channel]);
        let &(coefficient1, coefficient2) = coefficients
            .get(predictor)
            .ok_or(AudioDecodeError::InvalidData("invalid MS ADPCM predictor"))?;
        states.push(MsAdpcmState {
            coefficient1: i32::from(coefficient1),
            coefficient2: i32::from(coefficient2),
            delta: i32::from(read_u16(block, channels + channel * 2)?),
            sample1: read_i16(block, channels * 3 + channel * 2)?,
            sample2: read_i16(block, channels * 5 + channel * 2)?,
        });
    }

    let mut frames = Vec::with_capacity(2 + (block.len() - header_len) * 2 / channels);
    frames.push(stereo_i16_from(channels, |channel| states[channel].sample2));
    frames.push(stereo_i16_from(channels, |channel| states[channel].sample1));
    if channels == 1 {
        for byte in &block[header_len..] {
            for nibble in [byte >> 4, byte & 0x0f] {
                let sample = states[0].decode(nibble);
                frames.push(stereo_i16_from(1, |_| sample));
            }
        }
    } else {
        for byte in &block[header_len..] {
            let left = states[0].decode(byte >> 4);
            let right = states[1].decode(byte & 0x0f);
            frames.push(stereo_i16_from(
                2,
                |channel| if channel == 0 { left } else { right },
            ));
        }
    }
    Ok(frames)
}

struct MsAdpcmState {
    coefficient1: i32,
    coefficient2: i32,
    delta: i32,
    sample1: i16,
    sample2: i16,
}

impl MsAdpcmState {
    fn decode(&mut self, nibble: u8) -> i16 {
        let signed_nibble = if nibble & 0x08 == 0 {
            i32::from(nibble)
        } else {
            i32::from(nibble) - 16
        };
        let prediction = (i64::from(self.sample1) * i64::from(self.coefficient1)
            + i64::from(self.sample2) * i64::from(self.coefficient2))
            / 256
            + i64::from(signed_nibble) * i64::from(self.delta);
        let sample = prediction.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
        self.sample2 = self.sample1;
        self.sample1 = sample;
        self.delta =
            (self.delta * MS_ADPCM_ADAPTATION[usize::from(nibble)] / 256).clamp(16, 65_535);
        sample
    }
}

fn decode_ima_adpcm_block(
    block: &[u8],
    channels: usize,
) -> Result<Vec<[f32; 2]>, AudioDecodeError> {
    let header_len = 4 * channels;
    let group_len = 4 * channels;
    if block.len() < header_len || (block.len() - header_len) % group_len != 0 {
        return Err(AudioDecodeError::InvalidData("invalid IMA ADPCM block"));
    }
    let mut states = Vec::with_capacity(channels);
    for channel in 0..channels {
        let offset = channel * 4;
        states.push(ImaAdpcmState {
            sample: i32::from(read_i16(block, offset)?),
            // SDL reads this byte through a signed 8-bit field before clamping.
            index: i32::from(block[offset + 2] as i8).clamp(0, 88),
        });
    }

    let groups = (block.len() - header_len) / group_len;
    let mut frames = Vec::with_capacity(1 + groups * 8);
    frames.push(stereo_i16_from(channels, |channel| {
        states[channel].sample as i16
    }));
    let mut channel_samples = vec![[0_i16; 8]; channels];
    for group in 0..groups {
        let group_start = header_len + group * group_len;
        for channel in 0..channels {
            let channel_start = group_start + channel * 4;
            let mut output = 0;
            for byte in &block[channel_start..channel_start + 4] {
                for nibble in [byte & 0x0f, byte >> 4] {
                    channel_samples[channel][output] = states[channel].decode(nibble);
                    output += 1;
                }
            }
        }
        for sample in 0..8 {
            frames.push(stereo_i16_from(channels, |channel| {
                channel_samples[channel][sample]
            }));
        }
    }
    Ok(frames)
}

struct ImaAdpcmState {
    sample: i32,
    index: i32,
}

impl ImaAdpcmState {
    fn decode(&mut self, nibble: u8) -> i16 {
        let step = IMA_STEP_TABLE[self.index as usize];
        let mut difference = step >> 3;
        if nibble & 0x01 != 0 {
            difference += step >> 2;
        }
        if nibble & 0x02 != 0 {
            difference += step >> 1;
        }
        if nibble & 0x04 != 0 {
            difference += step;
        }
        if nibble & 0x08 != 0 {
            self.sample -= difference;
        } else {
            self.sample += difference;
        }
        self.sample = self.sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        self.index = (self.index + IMA_INDEX_ADJUSTMENT[usize::from(nibble)]).clamp(0, 88);
        self.sample as i16
    }
}

fn decode_a_law(byte: u8) -> i16 {
    let encoded = (byte & 0x7f) ^ 0x55;
    let exponent = (encoded & 0x70) >> 4;
    let mut sample = i32::from(encoded & 0x0f) << 4;
    sample += 8;
    if exponent != 0 {
        sample += 0x100;
    }
    if exponent > 1 {
        sample <<= exponent - 1;
    }
    if byte & 0x80 != 0 {
        sample as i16
    } else {
        (-sample) as i16
    }
}

fn decode_mu_law(byte: u8) -> i16 {
    let encoded = !byte;
    let exponent = (encoded & 0x70) >> 4;
    let mut sample = (i32::from(encoded & 0x0f) << 3) + 0x84;
    sample <<= exponent;
    if encoded & 0x80 != 0 {
        (0x84 - sample) as i16
    } else {
        (sample - 0x84) as i16
    }
}

fn read_stereo_frame(
    channels: usize,
    mut next_sample: impl FnMut() -> Result<Option<f32>, AudioDecodeError>,
) -> Result<Option<[f32; 2]>, AudioDecodeError> {
    let mut samples = [0.0_f32; MAX_MIXER_CHANNELS];
    for channel in 0..channels {
        let Some(sample) = next_sample()? else {
            return Ok(None);
        };
        samples[channel] = sample;
    }
    Ok(Some(sdl_stereo_mix(&samples[..channels])))
}

fn stereo_i16_from(channels: usize, mut sample: impl FnMut(usize) -> i16) -> [f32; 2] {
    let mut samples = [0.0_f32; MAX_MIXER_CHANNELS];
    for channel in 0..channels {
        samples[channel] = f32::from(sample(channel)) / f32::from(i16::MAX);
    }
    sdl_stereo_mix(&samples[..channels])
}

fn sdl_stereo_mix(samples: &[f32]) -> [f32; 2] {
    // These are SDL's channel-count layouts and normalization coefficients.
    // WAV channel masks are intentionally ignored by SDL's converter.
    match samples {
        [mono] => [*mono, *mono],
        [left, right] => [*left, *right],
        [front_left, front_right, lfe] => [
            0.800000012 * front_left + 0.200000003 * lfe,
            0.800000012 * front_right + 0.200000003 * lfe,
        ],
        [front_left, front_right, back_left, back_right] => [
            0.421000004 * front_left + 0.358999997 * back_left + 0.219999999 * back_right,
            0.421000004 * front_right + 0.219999999 * back_left + 0.358999997 * back_right,
        ],
        [front_left, front_right, lfe, back_left, back_right] => [
            0.374222219 * front_left
                + 0.111111112 * lfe
                + 0.319111109 * back_left
                + 0.195555553 * back_right,
            0.374222219 * front_right
                + 0.111111112 * lfe
                + 0.195555553 * back_left
                + 0.319111109 * back_right,
        ],
        [front_left, front_right, front_center, lfe, back_left, back_right] => [
            0.294545442 * front_left
                + 0.208181813 * front_center
                + 0.090909094 * lfe
                + 0.251818180 * back_left
                + 0.154545456 * back_right,
            0.294545442 * front_right
                + 0.208181813 * front_center
                + 0.090909094 * lfe
                + 0.154545456 * back_left
                + 0.251818180 * back_right,
        ],
        [front_left, front_right, front_center, lfe, back_center, side_left, side_right] => [
            0.247384623 * front_left
                + 0.174461529 * front_center
                + 0.076923080 * lfe
                + 0.174461529 * back_center
                + 0.226153851 * side_left
                + 0.100615382 * side_right,
            0.247384623 * front_right
                + 0.174461529 * front_center
                + 0.076923080 * lfe
                + 0.174461529 * back_center
                + 0.100615382 * side_left
                + 0.226153851 * side_right,
        ],
        [front_left, front_right, front_center, lfe, back_left, back_right, side_left, side_right] => {
            [
                0.211866662 * front_left
                    + 0.150266662 * front_center
                    + 0.066666670 * lfe
                    + 0.181066677 * back_left
                    + 0.111066669 * back_right
                    + 0.194133341 * side_left
                    + 0.085866667 * side_right,
                0.211866662 * front_right
                    + 0.150266662 * front_center
                    + 0.066666670 * lfe
                    + 0.111066669 * back_left
                    + 0.181066677 * back_right
                    + 0.085866667 * side_left
                    + 0.194133341 * side_right,
            ]
        }
        _ => unreachable!("validated WAV channel count"),
    }
}

fn extensible_subformat(fmt: &[u8]) -> Option<u16> {
    let guid = fmt.get(24..40)?;
    if guid[4..] != EXTENSIBLE_GUID_TAIL {
        return None;
    }
    let value = u32::from_le_bytes(guid[..4].try_into().ok()?);
    u16::try_from(value).ok()
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, AudioDecodeError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(AudioDecodeError::InvalidData("truncated WAV field"))?;
    Ok(u16::from_le_bytes(
        bytes.try_into().expect("two-byte slice"),
    ))
}

fn read_i16(data: &[u8], offset: usize) -> Result<i16, AudioDecodeError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(AudioDecodeError::InvalidData("truncated WAV field"))?;
    Ok(i16::from_le_bytes(
        bytes.try_into().expect("two-byte slice"),
    ))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, AudioDecodeError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(AudioDecodeError::InvalidData("truncated WAV field"))?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("four-byte slice"),
    ))
}

#[cfg(test)]
mod tests {
    use super::sdl_stereo_mix;

    fn assert_mix(samples: &[f32], expected: [f32; 2]) {
        let actual = sdl_stereo_mix(samples);
        assert!((actual[0] - expected[0]).abs() < 1.0e-7, "{actual:?}");
        assert!((actual[1] - expected[1]).abs() < 1.0e-7, "{actual:?}");
    }

    #[test]
    fn sdl_wav_multichannel_downmix_uses_count_layouts() {
        assert_mix(&[1.0, 0.0, 0.0], [0.800000012, 0.0]);
        assert_mix(&[0.0, 0.0, 1.0], [0.200000003, 0.200000003]);
        assert_mix(&[0.0, 0.0, 1.0, 0.0], [0.358999997, 0.219999999]);
        assert_mix(&[0.0, 0.0, 1.0, 0.0, 0.0], [0.111111112, 0.111111112]);
        assert_mix(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], [0.208181813, 0.208181813]);
        assert_mix(
            &[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [0.174461529, 0.174461529],
        );
        assert_mix(
            &[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            [0.181066677, 0.111066669],
        );
    }
}
