//! The native completion block passed to macOS microphone authorization.

use block2::RcBlock;
use objc2::runtime::Bool;
use std::sync::mpsc::Sender;

pub(crate) fn completion(reply: Sender<bool>) -> RcBlock<dyn Fn(Bool)> {
    RcBlock::new(move |granted: Bool| {
        // Capture may have been cancelled while the system prompt was open.
        let _ = reply.send(granted.as_bool());
    })
}
