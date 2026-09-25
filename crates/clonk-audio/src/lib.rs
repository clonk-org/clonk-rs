mod decoder;
mod fluidsynth;
mod midi;
mod mixer;
#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
mod sound_host;
mod tracker;
mod voice;
mod voice_capture_control;
mod voice_codec;
#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
mod voice_devices;
mod voice_microphone_test;
#[cfg(test)]
mod voice_qualification;
mod voice_resampling;
// Without the `cpal` feature nothing can open a microphone, so the capture-side
// halves of these two have no caller in that build. They still compile, and
// their tests still run.
#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
mod voice_echo;
#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
mod voice_output_reference;
#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
mod voice_processing;
mod wav;

pub use decoder::{decode_audio, AudioDecodeError, AudioFormat};
pub use mixer::{
    voice_playback_gain, AudioError, AudioOutputDevice, AudioOutputStats, AudioOutputStatus,
    AudioSystem, AudioWorkerHandle, ChannelId, MusicHandle, ResamplingMode, SoundHandle,
    VoiceFrameQueueOutcome, VoiceStreamStats, DEFAULT_VOICE_BUFFERED_FRAMES,
    MAX_VOICE_BUFFERED_FRAMES,
};
pub use voice::{
    voice_activation_level, voice_input_devices, VoiceCapture, VoiceCaptureError,
    VoiceCaptureOptions, VoiceCaptureStatus, VoiceCaptureTiming, VoiceInputDevice,
    VoiceInputDeviceId, VoiceInputDeviceIdParseError, VoiceInputFrame, VOICE_CAPTURE_QUEUE_FRAMES,
};
pub use voice_capture_control::VoiceCaptureControl;
#[cfg(any(test, feature = "test-hooks"))]
pub use voice_codec::test_encode_voice_frame;
pub use voice_codec::{
    EncodedVoiceFrame, VoiceCodecError, VoiceDecoder, VoiceEncoder, MAX_VOICE_ENCODED_BYTES,
    VOICE_FRAME_SAMPLES, VOICE_SAMPLE_RATE,
};
pub use voice_echo::VoiceEchoReference;
pub use voice_processing::{VoiceProcessingConfig, VoiceProcessingSwitches};

pub use sound_host::saved_device_follows_system_default;
pub use voice_devices::VoiceInputDeviceInventory;

pub use voice_microphone_test::{
    VoiceMicrophoneTest, VoiceMicrophoneTestState, VoiceMicrophoneTestStatus,
};
