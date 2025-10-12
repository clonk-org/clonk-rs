mod decoder;
pub mod ffi;
mod mixer;

pub use decoder::{decode_audio, AudioDecodeError, AudioFormat};
pub use mixer::{AudioError, AudioSystem, ChannelId, MusicHandle, SoundHandle};
