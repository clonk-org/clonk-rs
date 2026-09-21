//! A bounded receive lane with an independent budget for each speaker.

use std::collections::{BTreeMap, VecDeque};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{ClientId, VoiceFrame};
use tokio::sync::{
    mpsc::error::{TryRecvError, TrySendError},
    Notify,
};

const FRAMES_PER_SPEAKER: usize = 8;
const MAX_SPEAKERS: usize = 64;
const MAX_FRAME_AGE: Duration = Duration::from_millis(160);

/// Local receive time is preserved across every application-thread stall.
#[derive(Clone, Debug)]
pub struct ReceivedMedia<T> {
    pub frame: T,
    pub received_at: Instant,
}

pub trait InboxFrame {
    type Source: Copy + Ord + std::fmt::Debug;
    const MAX_SOURCES: usize = MAX_SPEAKERS;
    fn max_queued_frames(&self) -> usize {
        FRAMES_PER_SPEAKER
    }
    fn source(&self) -> Self::Source;
}

impl InboxFrame for VoiceFrame {
    type Source = ClientId;
    fn source(&self) -> ClientId {
        self.client_id
    }
}

#[derive(Debug)]
struct InboxState<T: InboxFrame> {
    queues: BTreeMap<T::Source, VecDeque<ReceivedMedia<T>>>,
    last_served: Option<T::Source>,
}

impl<T: InboxFrame> Default for InboxState<T> {
    fn default() -> Self {
        Self {
            queues: BTreeMap::new(),
            last_served: None,
        }
    }
}

#[derive(Debug)]
struct Inbox<T: InboxFrame> {
    state: Mutex<InboxState<T>>,
    ready: Notify,
    senders: AtomicUsize,
    receiver_closed: AtomicBool,
}

#[derive(Debug)]
pub struct MediaInboxSender<T: InboxFrame>(Arc<Inbox<T>>);
#[derive(Debug)]
pub struct MediaInboxReceiver<T: InboxFrame>(Arc<Inbox<T>>);

pub type VoiceInboxSender = MediaInboxSender<VoiceFrame>;
pub type VoiceInboxReceiver = MediaInboxReceiver<VoiceFrame>;
pub type ReceivedVoiceFrame = ReceivedMedia<VoiceFrame>;

pub fn voice_inbox() -> (VoiceInboxSender, VoiceInboxReceiver) {
    media_inbox()
}

pub(crate) fn media_inbox<T: InboxFrame>() -> (MediaInboxSender<T>, MediaInboxReceiver<T>) {
    let inbox = Arc::new(Inbox {
        state: Mutex::new(InboxState::default()),
        ready: Notify::new(),
        senders: AtomicUsize::new(1),
        receiver_closed: AtomicBool::new(false),
    });
    (MediaInboxSender(inbox.clone()), MediaInboxReceiver(inbox))
}

impl<T: InboxFrame> Clone for MediaInboxSender<T> {
    fn clone(&self) -> Self {
        self.0.senders.fetch_add(1, Ordering::Relaxed);
        Self(self.0.clone())
    }
}

impl<T: InboxFrame> Drop for MediaInboxSender<T> {
    fn drop(&mut self) {
        if self.0.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.ready.notify_waiters();
        }
    }
}

impl<T: InboxFrame> MediaInboxSender<T> {
    pub fn is_closed(&self) -> bool {
        self.0.receiver_closed.load(Ordering::Acquire)
    }

    pub fn try_send(&self, frame: T) -> Result<(), TrySendError<T>> {
        self.try_send_at(frame, Instant::now())
    }

    pub(crate) fn try_send_at(
        &self,
        frame: T,
        received_at: Instant,
    ) -> Result<(), TrySendError<T>> {
        if self.is_closed() {
            return Err(TrySendError::Closed(frame));
        }
        let capacity = frame
            .max_queued_frames()
            .clamp(1, FRAMES_PER_SPEAKER * MAX_SPEAKERS);
        // Media cannot block the session task or its lockstep commands.
        let Ok(mut state) = self.0.state.try_lock() else {
            return Err(TrySendError::Full(frame));
        };
        if !state.queues.contains_key(&frame.source()) {
            state.queues.retain(|_, frames| !frames.is_empty());
            if state.queues.len() >= T::MAX_SOURCES {
                return Err(TrySendError::Full(frame));
            }
        }
        let queue = state
            .queues
            .entry(frame.source())
            .or_insert_with(|| VecDeque::with_capacity(capacity));
        while queue.len() >= capacity {
            queue.pop_front();
        }
        queue.push_back(ReceivedMedia { frame, received_at });
        drop(state);
        self.0.ready.notify_one();
        Ok(())
    }
}

impl<T: InboxFrame> Drop for MediaInboxReceiver<T> {
    fn drop(&mut self) {
        self.0.receiver_closed.store(true, Ordering::Release);
    }
}

impl<T: InboxFrame> MediaInboxReceiver<T> {
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.try_recv_timed().map(|received| received.frame)
    }

    pub fn try_recv_timed(&mut self) -> Result<ReceivedMedia<T>, TryRecvError> {
        let Ok(mut state) = self.0.state.lock() else {
            return Err(TryRecvError::Empty);
        };
        let now = Instant::now();
        for queue in state.queues.values_mut() {
            queue.retain(|received| {
                now.saturating_duration_since(received.received_at) <= MAX_FRAME_AGE
            });
        }
        let after_previous = state.last_served.and_then(|last| {
            state
                .queues
                .range((Excluded(last), Unbounded))
                .find(|(_, frames)| !frames.is_empty())
                .map(|(source, _)| *source)
        });
        let source = after_previous.or_else(|| {
            state
                .queues
                .iter()
                .find(|(_, frames)| !frames.is_empty())
                .map(|(source, _)| *source)
        });
        if let Some(source) = source {
            state.last_served = Some(source);
            return state
                .queues
                .get_mut(&source)
                .and_then(VecDeque::pop_front)
                .ok_or(TryRecvError::Empty);
        }
        if self.0.senders.load(Ordering::Acquire) == 0 {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    pub async fn recv(&mut self) -> Option<T> {
        self.recv_timed().await.map(|received| received.frame)
    }

    pub async fn recv_timed(&mut self) -> Option<ReceivedMedia<T>> {
        loop {
            let shared = self.0.clone();
            let ready = shared.ready.notified();
            tokio::pin!(ready);
            ready.as_mut().enable();
            match self.try_recv_timed() {
                Ok(frame) => return Some(frame),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => ready.await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(client_id: u32, sequence: u16) -> VoiceFrame {
        VoiceFrame {
            client_id,
            player_id: client_id as i32,
            stream_epoch: 1,
            sequence,
            payload: vec![0; crate::MAX_VOICE_PAYLOAD_BYTES],
        }
    }

    #[test]
    fn busy_speaker_cannot_fill_another_speakers_inbox() {
        let (sender, mut receiver) = voice_inbox();
        for sequence in 0..8 {
            sender.try_send(frame(1, sequence)).unwrap();
        }
        sender
            .try_send(frame(2, 0))
            .expect("each speaker owns its queue budget");
        let first = receiver.try_recv().unwrap();
        let second = receiver.try_recv().unwrap();
        assert_ne!(
            first.client_id, second.client_id,
            "ready speakers receive fair service"
        );
    }

    #[test]
    fn overflow_keeps_a_speakers_freshest_frames() {
        let (sender, mut receiver) = voice_inbox();
        for sequence in 0..20 {
            sender.try_send(frame(1, sequence)).unwrap();
        }
        assert_eq!(receiver.try_recv().unwrap().sequence, 12);
        for sequence in 13..20 {
            assert_eq!(receiver.try_recv().unwrap().sequence, sequence);
        }
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn stalled_receiver_discards_old_speech_before_resuming() {
        let (sender, mut receiver) = voice_inbox();
        let now = Instant::now();
        sender
            .try_send_at(frame(1, 0), now - std::time::Duration::from_millis(200))
            .unwrap();
        sender.try_send_at(frame(2, 0), now).unwrap();
        let received = receiver.try_recv_timed().unwrap();
        assert_eq!(received.frame.client_id, 2);
        assert_eq!(received.received_at, now);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }
}
