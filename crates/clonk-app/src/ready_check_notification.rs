//! Actionable ready-check desktop notifications — the platform-independent core.
//!
//! The lobby's ready check already runs as an in-window Yes/No dialog with a
//! countdown (`game_app::lobby::tick_lobby_ready_check_prompt`); the desktop
//! notification beside it is passive. Making it actionable means an *answer*
//! can arrive from a backend callback thread while the same question is still
//! answerable in-window, so the two must not both resolve it.
//!
//! That race is the whole difficulty, and it is what this module owns:
//!
//! - [`ReadyCheckContinuation`] resolves **exactly once**, whichever side wins.
//!   A second activation, or one arriving after the dialog closed, timed out or
//!   the lobby tore down, is dropped rather than double-submitting to the live
//!   protocol request.
//! - Closing the continuation for any reason hides the notification, so a
//!   stale toast cannot answer a question that no longer exists.
//!
//! The platform backends are deliberately not here. They are developed where
//! they can be run — see the ticket — and reach this core through
//! [`NotificationSink`], which the tests drive with a fake.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A button on an actionable notification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NotificationAction {
    Yes,
    No,
}

/// The localized action labels a backend advertises.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NotificationActions {
    pub(crate) yes: String,
    pub(crate) no: String,
}

impl NotificationActions {
    pub(crate) fn label(&self, action: NotificationAction) -> &str {
        match action {
            NotificationAction::Yes => &self.yes,
            NotificationAction::No => &self.no,
        }
    }
}

/// What the user did with a shown notification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NotificationActivation {
    /// The notification body was clicked without choosing an action. Both
    /// platforms report this separately from the buttons, and it means "the
    /// user came back to the game", not "yes".
    Default,
    Chosen(NotificationAction),
}

/// A backend's handle to one shown notification.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct NotificationId(pub(crate) u32);

/// The platform seam. Every method may fail; failures are logged and ignored,
/// because a missing notification daemon must never take the lobby down.
pub(crate) trait NotificationSink {
    fn show(&self, actions: &NotificationActions) -> anyhow::Result<NotificationId>;
    fn hide(&self, id: NotificationId) -> anyhow::Result<()>;
}

/// The freedesktop notification protocol's action encoding.
///
/// `org.freedesktop.Notifications.Notify` takes actions as a **flat array of
/// alternating key and label** — `[key, label, key, label, ...]` — not as
/// pairs. Getting that interleaving wrong shows the key as the button text.
/// `"default"` is reserved: it has no button and fires when the body is
/// clicked, which is why [`NotificationActivation::Default`] exists.
pub(crate) const DEFAULT_ACTION_KEY: &str = "default";
pub(crate) const YES_ACTION_KEY: &str = "clonk-ready-yes";
pub(crate) const NO_ACTION_KEY: &str = "clonk-ready-no";

/// Builds the `Notify` actions array.
pub(crate) fn freedesktop_actions(actions: &NotificationActions) -> Vec<String> {
    vec![
        DEFAULT_ACTION_KEY.to_owned(),
        String::new(),
        YES_ACTION_KEY.to_owned(),
        actions.yes.clone(),
        NO_ACTION_KEY.to_owned(),
        actions.no.clone(),
    ]
}

/// Maps an `ActionInvoked(id, key)` signal onto an activation. An unknown key
/// is ignored rather than guessed at — a foreign key must never be read as an
/// answer.
pub(crate) fn activation_for_action_key(key: &str) -> Option<NotificationActivation> {
    match key {
        DEFAULT_ACTION_KEY => Some(NotificationActivation::Default),
        YES_ACTION_KEY => Some(NotificationActivation::Chosen(NotificationAction::Yes)),
        NO_ACTION_KEY => Some(NotificationActivation::Chosen(NotificationAction::No)),
        _ => None,
    }
}

/// Whether a `NotificationClosed(id, reason)` signal should close the
/// continuation.
///
/// Reasons are 1 expired, 2 dismissed by the user, 3 closed by
/// `CloseNotification`, 4 undefined. Only reason 3 is *our* doing — the
/// continuation is already resolved in that case — so it alone is ignored;
/// every other reason means the toast went away without an answer.
pub(crate) fn closed_reason_ends_prompt(reason: u32) -> bool {
    reason != 3
}

/// How a ready check finished.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadyCheckOutcome {
    /// An answer to submit. `Default` activation counts as no answer, so it
    /// never produces one.
    Answered(bool),
    /// The dialog closed, the countdown expired, or the lobby tore down.
    Closed,
}

/// The in-window ready-check continuation, resolvable from either thread.
///
/// Cloneable so a backend callback can hold one; the claim is shared.
#[derive(Clone)]
pub(crate) struct ReadyCheckContinuation {
    claimed: Arc<AtomicBool>,
    outcome: Arc<Mutex<Option<ReadyCheckOutcome>>>,
    shown: Arc<Mutex<Option<NotificationId>>>,
}

impl ReadyCheckContinuation {
    pub(crate) fn new() -> Self {
        Self {
            claimed: Arc::new(AtomicBool::new(false)),
            outcome: Arc::new(Mutex::new(None)),
            shown: Arc::new(Mutex::new(None)),
        }
    }

    /// Shows the toast for this continuation. A backend failure is non-fatal:
    /// the in-window dialog remains the answer path.
    pub(crate) fn show(&self, sink: &dyn NotificationSink, actions: &NotificationActions) {
        match sink.show(actions) {
            Ok(id) => {
                if let Ok(mut shown) = self.shown.lock() {
                    *shown = Some(id);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "ready-check notification could not be shown");
            }
        }
    }

    /// Whether this continuation has already been resolved.
    pub(crate) fn resolved(&self) -> bool {
        self.claimed.load(Ordering::Acquire)
    }

    /// The recorded outcome, once resolved.
    pub(crate) fn outcome(&self) -> Option<ReadyCheckOutcome> {
        self.outcome.lock().ok().and_then(|outcome| *outcome)
    }

    /// Claims the continuation. Returns `false` when someone already did — a
    /// late notification callback, a second button press, or a dialog answer
    /// racing a toast.
    fn claim(&self, outcome: ReadyCheckOutcome, sink: &dyn NotificationSink) -> bool {
        if self.claimed.swap(true, Ordering::AcqRel) {
            return false;
        }
        if let Ok(mut recorded) = self.outcome.lock() {
            *recorded = Some(outcome);
        }
        // Whatever resolved it, the toast is now stale.
        self.hide(sink);
        true
    }

    /// The in-window dialog answered. Returns whether this call owns the answer.
    pub(crate) fn answer(&self, ready: bool, sink: &dyn NotificationSink) -> bool {
        self.claim(ReadyCheckOutcome::Answered(ready), sink)
    }

    /// A notification activation arrived. `Default` closes without answering,
    /// matching what the platforms mean by clicking the body.
    pub(crate) fn activate(
        &self,
        activation: NotificationActivation,
        sink: &dyn NotificationSink,
    ) -> bool {
        let outcome = match activation {
            NotificationActivation::Chosen(NotificationAction::Yes) => {
                ReadyCheckOutcome::Answered(true)
            }
            NotificationActivation::Chosen(NotificationAction::No) => {
                ReadyCheckOutcome::Answered(false)
            }
            NotificationActivation::Default => ReadyCheckOutcome::Closed,
        };
        self.claim(outcome, sink)
    }

    /// The dialog closed, the countdown expired, or the lobby tore down. Any
    /// activation after this is ignored.
    pub(crate) fn close(&self, sink: &dyn NotificationSink) -> bool {
        self.claim(ReadyCheckOutcome::Closed, sink)
    }

    fn hide(&self, sink: &dyn NotificationSink) {
        let id = self.shown.lock().ok().and_then(|mut shown| shown.take());
        if let Some(id) = id {
            if let Err(error) = sink.hide(id) {
                tracing::warn!(%error, "ready-check notification could not be hidden");
            }
        }
    }
}

impl Default for ReadyCheckContinuation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeSink {
        shown: Mutex<Vec<NotificationId>>,
        hidden: Mutex<Vec<NotificationId>>,
        fail_show: bool,
        fail_hide: bool,
    }

    impl FakeSink {
        fn hidden(&self) -> Vec<NotificationId> {
            self.hidden.lock().expect("fake sink").clone()
        }

        fn shown(&self) -> Vec<NotificationId> {
            self.shown.lock().expect("fake sink").clone()
        }
    }

    impl NotificationSink for FakeSink {
        fn show(&self, _actions: &NotificationActions) -> anyhow::Result<NotificationId> {
            if self.fail_show {
                anyhow::bail!("no notification daemon");
            }
            let mut shown = self.shown.lock().expect("fake sink");
            let id = NotificationId(shown.len() as u32 + 1);
            shown.push(id);
            Ok(id)
        }

        fn hide(&self, id: NotificationId) -> anyhow::Result<()> {
            self.hidden.lock().expect("fake sink").push(id);
            if self.fail_hide {
                anyhow::bail!("notification already dismissed");
            }
            Ok(())
        }
    }

    fn actions() -> NotificationActions {
        NotificationActions {
            yes: "Ja".to_owned(),
            no: "Nein".to_owned(),
        }
    }

    #[test]
    fn ready_check_notification_actions_answer_and_dialog_close_hides_toast() {
        let labels = actions();
        assert_eq!(labels.label(NotificationAction::Yes), "Ja");
        assert_eq!(labels.label(NotificationAction::No), "Nein");

        // A Yes action answers the continuation and takes the toast down.
        let sink = FakeSink::default();
        let continuation = ReadyCheckContinuation::new();
        continuation.show(&sink, &labels);
        assert_eq!(sink.shown(), vec![NotificationId(1)]);
        assert!(!continuation.resolved());
        assert!(continuation.activate(
            NotificationActivation::Chosen(NotificationAction::Yes),
            &sink
        ));
        assert_eq!(
            continuation.outcome(),
            Some(ReadyCheckOutcome::Answered(true))
        );
        assert_eq!(sink.hidden(), vec![NotificationId(1)]);

        // A second activation resolves nothing and does not hide twice — this
        // is what stops a double submit to the live protocol request.
        assert!(!continuation.activate(
            NotificationActivation::Chosen(NotificationAction::No),
            &sink
        ));
        assert_eq!(
            continuation.outcome(),
            Some(ReadyCheckOutcome::Answered(true)),
            "the first resolution wins"
        );
        assert_eq!(sink.hidden(), vec![NotificationId(1)]);

        // No answers false.
        let sink = FakeSink::default();
        let continuation = ReadyCheckContinuation::new();
        continuation.show(&sink, &labels);
        assert!(continuation.activate(
            NotificationActivation::Chosen(NotificationAction::No),
            &sink
        ));
        assert_eq!(
            continuation.outcome(),
            Some(ReadyCheckOutcome::Answered(false))
        );

        // Clicking the body is "come back to the game", not "yes": it closes
        // without submitting an answer.
        let sink = FakeSink::default();
        let continuation = ReadyCheckContinuation::new();
        continuation.show(&sink, &labels);
        assert!(continuation.activate(NotificationActivation::Default, &sink));
        assert_eq!(continuation.outcome(), Some(ReadyCheckOutcome::Closed));
        assert_eq!(sink.hidden(), vec![NotificationId(1)]);

        // The in-window dialog answering hides the toast...
        let sink = FakeSink::default();
        let continuation = ReadyCheckContinuation::new();
        continuation.show(&sink, &labels);
        assert!(continuation.answer(true, &sink));
        assert_eq!(sink.hidden(), vec![NotificationId(1)]);
        // ...and a toast activation arriving afterwards is dropped.
        assert!(!continuation.activate(
            NotificationActivation::Chosen(NotificationAction::No),
            &sink
        ));
        assert_eq!(
            continuation.outcome(),
            Some(ReadyCheckOutcome::Answered(true))
        );

        // Timeout or teardown closes the same way, and blocks late callbacks.
        let sink = FakeSink::default();
        let continuation = ReadyCheckContinuation::new();
        continuation.show(&sink, &labels);
        assert!(continuation.close(&sink));
        assert_eq!(sink.hidden(), vec![NotificationId(1)]);
        assert!(!continuation.close(&sink));
        assert!(!continuation.answer(true, &sink));
        assert_eq!(continuation.outcome(), Some(ReadyCheckOutcome::Closed));

        // A backend that cannot show anything is non-fatal: the continuation
        // still answers from the in-window dialog, and hides nothing.
        let sink = FakeSink {
            fail_show: true,
            ..FakeSink::default()
        };
        let continuation = ReadyCheckContinuation::new();
        continuation.show(&sink, &labels);
        assert!(sink.shown().is_empty());
        assert!(continuation.answer(false, &sink));
        assert!(sink.hidden().is_empty());
        assert_eq!(
            continuation.outcome(),
            Some(ReadyCheckOutcome::Answered(false))
        );

        // A backend that fails to hide is non-fatal too — the continuation is
        // still resolved.
        let sink = FakeSink {
            fail_hide: true,
            ..FakeSink::default()
        };
        let continuation = ReadyCheckContinuation::new();
        continuation.show(&sink, &labels);
        assert!(continuation.answer(true, &sink));
        assert_eq!(sink.hidden(), vec![NotificationId(1)]);
        assert_eq!(
            continuation.outcome(),
            Some(ReadyCheckOutcome::Answered(true))
        );
    }

    // The freedesktop wire encoding — the part of a Linux backend that is
    // testable without a bus, and the part that is easy to get wrong.
    #[test]
    fn freedesktop_actions_interleave_keys_and_labels_with_a_default_entry() {
        let encoded = freedesktop_actions(&actions());
        assert_eq!(
            encoded,
            vec![
                "default".to_owned(),
                String::new(),
                "clonk-ready-yes".to_owned(),
                "Ja".to_owned(),
                "clonk-ready-no".to_owned(),
                "Nein".to_owned(),
            ],
            "Notify takes a flat key,label,key,label array — not pairs"
        );
        // Labels sit at the odd indices; a swapped pair would show the key as
        // the button text.
        assert_eq!(encoded[3], actions().yes);
        assert_eq!(encoded[5], actions().no);

        assert_eq!(
            activation_for_action_key("clonk-ready-yes"),
            Some(NotificationActivation::Chosen(NotificationAction::Yes))
        );
        assert_eq!(
            activation_for_action_key("clonk-ready-no"),
            Some(NotificationActivation::Chosen(NotificationAction::No))
        );
        assert_eq!(
            activation_for_action_key("default"),
            Some(NotificationActivation::Default)
        );
        assert_eq!(
            activation_for_action_key("some-other-app"),
            None,
            "a foreign action key must never be read as an answer"
        );

        // NotificationClosed: only reason 3 is our own CloseNotification, so
        // only that one leaves the continuation alone.
        assert!(closed_reason_ends_prompt(1), "expired");
        assert!(closed_reason_ends_prompt(2), "dismissed by the user");
        assert!(
            !closed_reason_ends_prompt(3),
            "closed by us — already resolved"
        );
        assert!(closed_reason_ends_prompt(4), "undefined");
    }

    #[test]
    fn concurrent_ready_check_resolution_submits_exactly_one_answer() {
        // Criterion 2's real content: the dialog thread and a backend callback
        // thread race, and exactly one of them owns the answer.
        for _ in 0..64 {
            let sink = Arc::new(FakeSink::default());
            let continuation = ReadyCheckContinuation::new();
            continuation.show(sink.as_ref(), &actions());

            let winners: Vec<bool> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..4)
                    .map(|index| {
                        let continuation = continuation.clone();
                        let sink = Arc::clone(&sink);
                        scope.spawn(move || {
                            if index % 2 == 0 {
                                continuation.answer(true, sink.as_ref())
                            } else {
                                continuation.activate(
                                    NotificationActivation::Chosen(NotificationAction::No),
                                    sink.as_ref(),
                                )
                            }
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("resolver thread"))
                    .collect()
            });

            assert_eq!(
                winners.iter().filter(|won| **won).count(),
                1,
                "exactly one resolver may submit"
            );
            assert_eq!(
                sink.hidden().len(),
                1,
                "the toast is hidden exactly once, by the winner"
            );
        }
    }
}
