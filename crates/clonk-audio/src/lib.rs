mod decoder;
mod fluidsynth;
mod midi;
mod mixer;
mod tracker;
mod voice;
// Without the `cpal` feature nothing can open a microphone, so the capture-side
// halves of these two have no caller in that build. They still compile, and
// their tests still run.
#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
mod voice_echo;
#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
mod voice_processing;
mod wav;

pub use decoder::{decode_audio, AudioDecodeError, AudioFormat};
pub use mixer::{
    AudioError, AudioSystem, ChannelId, MusicHandle, ResamplingMode, SoundHandle,
    VoiceFrameQueueOutcome, VoiceStreamStats, DEFAULT_VOICE_BUFFERED_FRAMES,
    MAX_VOICE_BUFFERED_FRAMES,
};
pub use voice::{
    decode_voice_frame, encode_voice_frame, voice_activation_level, EncodedVoiceFrame,
    VoiceCapture, VoiceCaptureError, VoiceCaptureOptions, VoiceCodecError, VoiceInputFrame,
    VOICE_CAPTURE_QUEUE_FRAMES, VOICE_ENCODED_FRAME_BYTES, VOICE_FRAME_SAMPLES, VOICE_SAMPLE_RATE,
};
pub use voice_echo::VoiceEchoReference;
pub use voice_processing::{VoiceProcessingConfig, VoiceProcessingSwitches};
