//! Bounded, stateful Opus for the versioned voice media lane.

pub const VOICE_SAMPLE_RATE: u32 = 48_000;
pub const VOICE_FRAME_SAMPLES: usize = 960;
pub const MAX_VOICE_ENCODED_BYTES: usize = 512;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_rejects_network_silence_and_stereo_before_changing_state() {
        let mut decoder = VoiceDecoder::new().unwrap();
        assert!(
            decoder.decode(&[], false).is_err(),
            "loss concealment is explicit"
        );
        let mut stereo = opus::Encoder::new(
            VOICE_SAMPLE_RATE,
            opus::Channels::Stereo,
            opus::Application::Audio,
        )
        .unwrap();
        let packet = stereo
            .encode_vec(&[1_000; VOICE_FRAME_SAMPLES * 2], MAX_VOICE_ENCODED_BYTES)
            .unwrap();
        assert!(
            decoder.decode(&packet, false).is_err(),
            "remote peers cannot change the negotiated channel count"
        );
    }

    #[test]
    fn opus_preserves_fullband_audio_in_bounded_twenty_millisecond_packets() {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut decoder = VoiceDecoder::new().unwrap();
        let mut output = Vec::new();
        for frame in 0..50 {
            let samples = std::array::from_fn(|sample| {
                let t = (frame * VOICE_FRAME_SAMPLES + sample) as f32 / VOICE_SAMPLE_RATE as f32;
                ((std::f32::consts::TAU * 12_000.0 * t).sin() * 12_000.0
                    + (std::f32::consts::TAU * 200.0 * t).sin() * 8_000.0) as i16
            });
            let packet = encoder.encode(&samples).unwrap();
            assert!(!packet.is_empty());
            assert!(packet.len() <= MAX_VOICE_ENCODED_BYTES);
            output.extend(decoder.decode(&packet, false).unwrap());
        }
        // A component above the old codec's 8 kHz Nyquist must survive.
        let settled = &output[VOICE_FRAME_SAMPLES * 10..];
        let (sin, cos) =
            settled
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(sin, cos), (index, &sample)| {
                    let phase = std::f64::consts::TAU * 12_000.0 * index as f64
                        / f64::from(VOICE_SAMPLE_RATE);
                    (
                        sin + f64::from(sample) * phase.sin(),
                        cos + f64::from(sample) * phase.cos(),
                    )
                });
        let amplitude = 2.0 * sin.hypot(cos) / settled.len() as f64;
        assert!(
            amplitude > 6_000.0,
            "fullband component was lost: amplitude {amplitude}"
        );
    }
}

/// A complete encoded frame. The fixed storage bounds allocation and wire size;
/// only the initialized prefix is transmitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodedVoiceFrame {
    bytes: [u8; MAX_VOICE_ENCODED_BYTES],
    length: usize,
}

impl std::ops::Deref for EncodedVoiceFrame {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

impl EncodedVoiceFrame {
    pub fn from_packet(packet: &[u8]) -> Result<Self, VoiceCodecError> {
        validate_voice_packet(packet)?;
        let mut frame = Self {
            bytes: [0; MAX_VOICE_ENCODED_BYTES],
            length: packet.len(),
        };
        frame.bytes[..packet.len()].copy_from_slice(packet);
        Ok(frame)
    }
}

/// Validate geometry before a network packet reaches a stateful decoder.
pub fn validate_voice_packet(packet: &[u8]) -> Result<(), VoiceCodecError> {
    if packet.is_empty()
        || packet.len() > MAX_VOICE_ENCODED_BYTES
        || opus::packet::get_nb_channels(packet)? != opus::Channels::Mono
        || opus::packet::get_nb_samples(packet, VOICE_SAMPLE_RATE)? != VOICE_FRAME_SAMPLES
    {
        return Err(VoiceCodecError::InvalidPacket);
    }
    opus::packet::parse(packet)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceCodecError {
    #[error("voice packet is not a bounded 20 ms mono Opus frame")]
    InvalidPacket,
    #[error("Opus voice processing failed: {0}")]
    Codec(#[from] opus::Error),
}

#[derive(Debug)]
pub struct VoiceEncoder(opus::Encoder);

impl VoiceEncoder {
    /// Codec lookahead, used when flushing an utterance and measuring fidelity.
    pub fn lookahead_samples(&mut self) -> Result<usize, VoiceCodecError> {
        Ok(self.0.get_lookahead()? as usize)
    }

    pub fn new() -> Result<Self, VoiceCodecError> {
        let mut encoder = opus::Encoder::new(
            VOICE_SAMPLE_RATE,
            opus::Channels::Mono,
            opus::Application::Voip,
        )?;
        encoder.set_bitrate(opus::Bitrate::Bits(48_000))?;
        encoder.set_bandwidth(opus::Bandwidth::Fullband)?;
        encoder.set_complexity(5)?;
        encoder.set_inband_fec(true)?;
        encoder.set_packet_loss_perc(10)?;
        encoder.set_dtx(true)?;
        Ok(Self(encoder))
    }

    pub fn encode(
        &mut self,
        samples: &[i16; VOICE_FRAME_SAMPLES],
    ) -> Result<EncodedVoiceFrame, VoiceCodecError> {
        let mut packet = EncodedVoiceFrame {
            bytes: [0; MAX_VOICE_ENCODED_BYTES],
            length: 0,
        };
        packet.length = self.0.encode(samples, &mut packet.bytes)?;
        Ok(packet)
    }
}

#[derive(Debug)]
pub struct VoiceDecoder(opus::Decoder);

impl VoiceDecoder {
    pub fn new() -> Result<Self, VoiceCodecError> {
        Ok(Self(opus::Decoder::new(
            VOICE_SAMPLE_RATE,
            opus::Channels::Mono,
        )?))
    }

    /// Decode in playout order. `fec` recovers the preceding missing frame
    /// from a successor; decode that same successor normally on its own turn.
    pub fn decode(
        &mut self,
        packet: &[u8],
        fec: bool,
    ) -> Result<[i16; VOICE_FRAME_SAMPLES], VoiceCodecError> {
        validate_voice_packet(packet)?;
        self.decode_inner(packet, fec)
    }

    fn decode_inner(
        &mut self,
        packet: &[u8],
        fec: bool,
    ) -> Result<[i16; VOICE_FRAME_SAMPLES], VoiceCodecError> {
        let mut samples = [0; VOICE_FRAME_SAMPLES];
        let count = self.0.decode(packet, &mut samples, fec)?;
        if count != VOICE_FRAME_SAMPLES {
            return Err(VoiceCodecError::InvalidPacket);
        }
        Ok(samples)
    }

    /// Codec-native loss concealment, always exactly one media interval.
    pub fn conceal(&mut self) -> Result<[i16; VOICE_FRAME_SAMPLES], VoiceCodecError> {
        self.decode_inner(&[], false)
    }
}

/// Convenience for synthetic test fixtures. Live capture retains its encoder.
#[cfg(any(test, feature = "test-hooks"))]
pub fn test_encode_voice_frame(
    samples: &[i16; VOICE_FRAME_SAMPLES],
) -> Result<EncodedVoiceFrame, VoiceCodecError> {
    VoiceEncoder::new()?.encode(samples)
}
