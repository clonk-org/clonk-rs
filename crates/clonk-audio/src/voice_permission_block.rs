//! The native completion block passed to macOS microphone authorization.

use block2::{ManualBlockEncoding, RcBlock};
use objc2::runtime::Bool;
use std::ffi::CStr;
use std::sync::mpsc::Sender;

struct MicrophoneCompletionEncoding;

// SAFETY: this is Clang's encoding for void (^)(BOOL) on the two supported
// macOS architectures: BOOL is _Bool on ARM64 and signed char on Intel.
unsafe impl ManualBlockEncoding for MicrophoneCompletionEncoding {
    type Arguments = (Bool,);
    type Return = ();
    const ENCODING_CSTR: &'static CStr = if cfg!(target_arch = "aarch64") {
        c"v12@?0B8"
    } else {
        c"v12@?0c8"
    };
}

pub(crate) fn completion(reply: Sender<bool>) -> RcBlock<dyn Fn(Bool)> {
    // RcBlock::new omits the runtime signature. Native APIs that inspect their
    // completion blocks can abort before presenting the permission dialog.
    RcBlock::with_encoding::<_, _, _, MicrophoneCompletionEncoding>(move |granted: Bool| {
        // Capture may have been cancelled while the system prompt was open.
        let _ = reply.send(granted.as_bool());
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::encode::Encode;

    #[test]
    fn microphone_completion_has_a_runtime_signature_for_native_introspection() {
        let (reply, _) = std::sync::mpsc::channel();
        let block = completion(reply);
        // Clang's Apple Blocks ABI requires a descriptor signature when
        // BLOCK_HAS_SIGNATURE is set. BOOL encodes differently on Intel/ARM.
        // https://clang.llvm.org/docs/Block-ABI-Apple.html#high-level
        let encoding = format!("v12@?0{}8", Bool::ENCODING);
        let descriptor = format!("{block:?}");
        assert!(
            descriptor.contains(&format!("encoding: Some({encoding:?})")),
            "microphone authorization must receive encoding {encoding:?}: {descriptor}",
        );
    }

    #[test]
    fn microphone_completion_copy_delivers_decisions_after_the_original_is_dropped() {
        for granted in [true, false] {
            let (reply, response) = std::sync::mpsc::channel();
            let original = completion(reply);
            let copied = original.clone();
            drop(original);
            copied.call((Bool::new(granted),));
            assert_eq!(response.try_recv().unwrap(), granted);
            // The framework may finish after capture and its receiver are gone.
            drop(response);
            copied.call((Bool::new(granted),));
        }
    }
}
