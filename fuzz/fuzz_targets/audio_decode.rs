//! WAV, MIDI and RMID through the production sniff-and-dispatch entry point
//! (clonk-org/clonk-rs#964).
//!
//! The amplification risk here is numeric rather than textual: RIFF chunk
//! lengths, WAV channel and sample-rate fields, MIDI variable-length quantities
//! and tempo changes are all values that *multiply*, so a few bytes can name a
//! great deal of decoded output.
//!
//! A MIDI decode is expensive — the synthesizer runs to its liveness bound even
//! for an empty timeline (`fluidsynth.rs:18-21`) — which is why the ordinary
//! suite keeps only two MIDI seeds and the richer set lives in this target's
//! corpus.

#![no_main]

use clonk_audio::decode_audio;
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's cap, so a finding here reproduces there.
const MAX_INPUT: usize = 4096;

/// `MAX_EAGER_DECODE_SECONDS` in `fluidsynth.rs`: the decoder's own limit, not
/// one invented here.
const MAX_MIDI_SECONDS: usize = 15 * 60;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }

    if let Ok(decoded) = decode_audio(data) {
        // PCM output is proportional to input — 4-bit ADPCM is the densest
        // legacy codec — while a MIDI timeline is bounded by duration instead.
        let pcm_ceiling = data.len() * 8 + 4096;
        let midi_ceiling = MAX_MIDI_SECONDS * decoded.sample_rate.max(1) as usize;
        assert!(
            decoded.frames.len() <= pcm_ceiling.max(midi_ceiling),
            "decoded {} frames from {} bytes at {} Hz",
            decoded.frames.len(),
            data.len(),
            decoded.sample_rate
        );
    }
});
