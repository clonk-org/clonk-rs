//! Platform backends for the actionable ready-check notification.
//!
//! The platform-independent core — the atomic claim, the action-key mapping
//! and the signal routing — lives in [`crate::ready_check_notification`]. Only
//! the translation between one platform's notification API and that core lives
//! here, so the race the core owns is never re-implemented per platform.

use std::sync::Arc;
use std::thread::JoinHandle;

use crate::desktop_notification::DesktopNotification;
use crate::ready_check_notification::{
    NotificationActions, NotificationSink, ReadyCheckContinuation,
};

/// A backend that shows an actionable toast and routes its buttons into the
/// continuation that owns the prompt.
///
/// `NotificationSink` is the half the app keeps: `ReadyCheckDialog::OnClosed`
/// hides the toast from whichever side resolved the prompt
/// (`src/C4Network2.cpp:176-178`), the in-window dialog on the app thread
/// included, so the object the watcher shows through must be the object the
/// app hides through.
pub(crate) trait ActionableSink: NotificationSink + Send + Sync {
    /// Shows the toast and watches it until it is gone.
    ///
    /// The freedesktop backend blocks reading the bus until the toast no
    /// longer exists; the WinRT backend returns once its handlers are attached
    /// and the platform delivers the answer later. Either way the caller runs
    /// it on its own thread, so a resolved prompt never waits on a daemon.
    fn show_and_watch(
        &self,
        actions: &NotificationActions,
        continuation: &ReadyCheckContinuation,
    ) -> anyhow::Result<()>;
}

/// The platform's actionable sink for one prompt, or `None` where no
/// notification service exists: macOS and the non-desktop targets.
///
/// A backend that fails to initialise is reported and treated the same, so
/// the caller leaves `SilentSink` in place and the in-window dialog stays the
/// answer path (clonk-org/clonk-rs#1308).
/// How the lobby obtains a desktop toast whose buttons answer the ready check.
///
/// The app owns this rather than calling [`platform_sink`] directly so the
/// choice is made at the composition root: production asks the platform, and
/// a test that wants the actionable branch injects a sink of its own.
pub(crate) struct ReadyCheckToastBackend(Box<SinkChooser>);

type SinkChooser = dyn Fn(&DesktopNotification) -> Option<Arc<dyn ActionableSink>>;

impl ReadyCheckToastBackend {
    /// The sink to show `notification` through, if this desktop has one.
    pub(crate) fn sink_for(
        &self,
        notification: &DesktopNotification,
    ) -> Option<Arc<dyn ActionableSink>> {
        (self.0)(notification)
    }

    /// A backend that always offers `sink`.
    #[cfg(test)]
    pub(crate) fn with_sink(sink: Arc<dyn ActionableSink>) -> Self {
        Self(Box::new(move |_| Some(Arc::clone(&sink))))
    }
}

impl Default for ReadyCheckToastBackend {
    #[cfg(not(test))]
    fn default() -> Self {
        Self(Box::new(platform_sink))
    }

    /// Under test the default offers no actionable sink on any platform, so
    /// the app suite observes the notification queue the app owns everywhere.
    /// Without this a Linux run took the freedesktop branch and a test that
    /// expected the queued toast passed on macOS and failed on the Linux
    /// runner. A test that wants the actionable branch injects its own sink
    /// with [`ReadyCheckToastBackend::with_sink`].
    #[cfg(test)]
    fn default() -> Self {
        Self(Box::new(|_| None))
    }
}

pub(crate) fn platform_sink(notification: &DesktopNotification) -> Option<Arc<dyn ActionableSink>> {
    #[cfg(target_os = "linux")]
    {
        match freedesktop::FreedesktopSink::new(notification.clone()) {
            Ok(sink) => Some(Arc::new(sink)),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "ready-check toast backend is unavailable; the in-window dialog answers"
                );
                None
            }
        }
    }
    #[cfg(windows)]
    {
        match winrt::WinRtSink::for_notification(notification.clone()) {
            Ok(sink) => Some(Arc::new(sink)),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "ready-check toast backend is unavailable; the in-window dialog answers"
                );
                None
            }
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = notification;
        None
    }
}

/// Runs `show_and_watch` on its own thread.
///
/// The thread holds a clone of the continuation, whose claim is shared, so an
/// answer from the toast and an answer from the dialog race safely; and it
/// holds the same sink the app keeps, so whichever side wins hides the toast
/// the other showed. A failure to start the thread is the caller's to log:
/// the prompt is still answerable in the window.
pub(crate) fn watch_on_thread(
    sink: Arc<dyn ActionableSink>,
    actions: NotificationActions,
    continuation: ReadyCheckContinuation,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("ready-check-toast".to_owned())
        .spawn(move || {
            if let Err(error) = sink.show_and_watch(&actions, &continuation) {
                tracing::warn!(
                    %error,
                    "ready-check toast listener failed; the in-window dialog answers"
                );
            }
        })
}

#[cfg(target_os = "linux")]
pub(crate) mod freedesktop {
    use std::collections::HashMap;

    use anyhow::{Context, Result};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::Value;

    use crate::desktop_notification::DesktopNotification;
    use crate::ready_check_notification::{
        dispatch_signal, freedesktop_actions, NotificationActions, NotificationId,
        NotificationSignal, NotificationSink, ReadyCheckContinuation,
    };

    const SERVICE: &str = "org.freedesktop.Notifications";
    const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
    const APPLICATION_NAME: &str = "Clonk Rust";

    /// An actionable ready-check toast on the freedesktop notification service.
    pub(crate) struct FreedesktopSink {
        connection: Connection,
        notification: DesktopNotification,
    }

    impl FreedesktopSink {
        pub(crate) fn new(notification: DesktopNotification) -> Result<Self> {
            let connection = Connection::session()
                .context("failed to connect to the desktop notification session bus")?;
            Ok(Self {
                connection,
                notification,
            })
        }

        fn proxy(&self) -> Result<Proxy<'_>> {
            Proxy::new(&self.connection, SERVICE, OBJECT_PATH, SERVICE)
                .context("failed to create the desktop notification proxy")
        }

        /// Shows the toast and reads signals until it is gone.
        ///
        /// Blocking, so a caller runs it on its own thread.
        ///
        /// The subscription is taken *before* `Notify`, and that order is
        /// load-bearing: the bus delivers only signals emitted after a match
        /// rule is in place, so subscribing afterwards drops a button pressed
        /// in the instant the toast appears — the press most likely to happen,
        /// since that is when the user is looking at it.
        ///
        /// It returns when [`dispatch_signal`] reports the toast no longer
        /// exists — including the `NotificationClosed` that our own
        /// `CloseNotification` produces. That is what makes an *unanswered*
        /// prompt's listener exit when the lobby tears the continuation down,
        /// rather than blocking on a bus that will never mention it again.
        pub(crate) fn show_and_watch(
            &self,
            actions: &NotificationActions,
            continuation: &ReadyCheckContinuation,
        ) -> Result<()> {
            let signals = self
                .proxy()?
                .receive_all_signals()
                .context("failed to subscribe to desktop notification signals")?;
            continuation.show(self, actions);
            // No toast means no daemon, which is non-fatal by design: the
            // in-window dialog stays the answer path, and there is nothing to
            // watch for.
            let Some(id) = continuation.shown_id() else {
                return Ok(());
            };
            for message in signals {
                if parse_signal(&message, id)
                    .is_some_and(|signal| dispatch_signal(&signal, continuation, self))
                {
                    break;
                }
            }
            Ok(())
        }
    }

    /// Parses one bus message into a signal for `id`, or `None` when it is not
    /// one of ours.
    ///
    /// Both the notification id and the message member are filtered here: the
    /// service broadcasts every application's activations on the same
    /// connection, so an unfiltered listener would read a stranger's button
    /// press as an answer to this prompt.
    fn parse_signal(message: &zbus::Message, id: NotificationId) -> Option<NotificationSignal> {
        let header = message.header();
        let body = message.body();
        match header.member()?.as_str() {
            "ActionInvoked" => body
                .deserialize::<(u32, String)>()
                .ok()
                .filter(|(signal_id, _)| *signal_id == id.0)
                .map(|(_, key)| NotificationSignal::ActionInvoked(key)),
            "NotificationClosed" => body
                .deserialize::<(u32, u32)>()
                .ok()
                .filter(|(signal_id, _)| *signal_id == id.0)
                .map(|(_, reason)| NotificationSignal::Closed(reason)),
            _ => None,
        }
    }

    impl super::ActionableSink for FreedesktopSink {
        fn show_and_watch(
            &self,
            actions: &NotificationActions,
            continuation: &ReadyCheckContinuation,
        ) -> Result<()> {
            FreedesktopSink::show_and_watch(self, actions, continuation)
        }
    }

    impl NotificationSink for FreedesktopSink {
        fn show(&self, actions: &NotificationActions) -> Result<NotificationId> {
            let expiration =
                i32::try_from(self.notification.expiration.as_millis()).unwrap_or(i32::MAX);
            let hints = HashMap::<&str, Value<'_>>::new();
            let id: u32 = self
                .proxy()?
                .call(
                    "Notify",
                    &(
                        APPLICATION_NAME,
                        0_u32,
                        "",
                        self.notification.title.as_str(),
                        self.notification.body.as_str(),
                        freedesktop_actions(actions),
                        hints,
                        expiration,
                    ),
                )
                .context("desktop notification service rejected the ready check")?;
            Ok(NotificationId(id))
        }

        fn hide(&self, id: NotificationId) -> Result<()> {
            self.proxy()?
                .call::<_, _, ()>("CloseNotification", &(id.0,))
                .context("failed to close the ready-check notification")?;
            Ok(())
        }
    }
}

#[cfg(windows)]
pub(crate) mod winrt {
    use anyhow::{anyhow, Context, Result};
    use windows::{
        core::{IInspectable, Interface, Ref, HSTRING},
        Data::Xml::Dom::{XmlDocument, XmlElement},
        Foundation::TypedEventHandler,
        UI::Notifications::{
            ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs,
            ToastNotification, ToastNotifier,
        },
    };

    use crate::desktop_notification::{
        backend::{create_toast_notifier, toast_content, toast_expiration},
        DesktopNotification,
    };
    use crate::ready_check_notification::{
        dispatch_signal, NotificationActions, NotificationId, NotificationSignal, NotificationSink,
        ReadyCheckContinuation, DEFAULT_ACTION_KEY, NO_ACTION_KEY, YES_ACTION_KEY,
    };

    /// WinRT reports dismissals as an enum; the shared core reads freedesktop's
    /// numbering. Translating here rather than widening the core keeps one
    /// definition of "did *we* close this?" — the distinction that decides
    /// whether a close ends the prompt.
    ///
    /// `ApplicationHidden` is our own `Hide`, so it maps to freedesktop reason
    /// 3, the one reason `closed_reason_ends_prompt` deliberately ignores.
    fn freedesktop_reason(reason: ToastDismissalReason) -> u32 {
        match reason {
            ToastDismissalReason::ApplicationHidden => 3,
            ToastDismissalReason::UserCanceled => 2,
            ToastDismissalReason::TimedOut => 1,
            _ => 4,
        }
    }

    /// The sink a *callback* resolves through.
    ///
    /// `hide` is a no-op on purpose. A callback only runs because the toast is
    /// already going away, and WinRT has no id-addressed close to issue for one
    /// that has gone. Capturing the live `ToastNotification` here to call
    /// `Hide` would also make the toast own the handler that owns the toast —
    /// a reference cycle that outlives the prompt.
    struct DismissedToast;

    impl NotificationSink for DismissedToast {
        fn show(&self, _actions: &NotificationActions) -> Result<NotificationId> {
            Err(anyhow!("a ready-check callback cannot show a notification"))
        }

        fn hide(&self, _id: NotificationId) -> Result<()> {
            Ok(())
        }
    }

    /// An actionable ready-check toast on the WinRT notification manager.
    ///
    /// Unlike the freedesktop backend there is no listener thread: WinRT
    /// delivers activation and dismissal through handlers registered on the
    /// toast. They may arrive on a thread other than the one that showed it,
    /// which is exactly what [`ReadyCheckContinuation`]'s atomic claim exists
    /// to make safe.
    pub(crate) struct WinRtSink {
        notifier: ToastNotifier,
        /// Title, body and expiration of the toast this sink shows.
        notification: Option<DesktopNotification>,
        shown: std::sync::Mutex<Option<ToastNotification>>,
    }

    impl WinRtSink {
        pub(crate) fn new(notifier: ToastNotifier) -> Self {
            Self {
                notifier,
                notification: None,
                shown: std::sync::Mutex::new(None),
            }
        }

        /// A sink for one prompt, on the application's toast notifier. The
        /// plain notifier initialised the WinRT apartment at startup; a
        /// process where that failed fails here too, which the caller treats
        /// as "no backend".
        pub(crate) fn for_notification(notification: DesktopNotification) -> Result<Self> {
            let mut sink = Self::new(create_toast_notifier()?);
            sink.notification = Some(notification);
            Ok(sink)
        }

        /// Shows the toast and routes its callbacks into `continuation`.
        ///
        /// The freedesktop backend reaches the same place by reading the bus on
        /// its own thread; here the platform does the waiting for us.
        pub(crate) fn show_and_watch(
            &self,
            content: &XmlDocument,
            actions: &NotificationActions,
            continuation: &ReadyCheckContinuation,
        ) -> Result<NotificationId> {
            attach_actions(content, actions)?;
            let toast = ToastNotification::CreateToastNotification(content)
                .context("failed to create the actionable WinRT toast")?;
            if let Some(notification) = self.notification.as_ref() {
                toast
                    .SetExpirationTime(&toast_expiration(notification.expiration)?)
                    .context("failed to set the actionable WinRT toast expiration")?;
            }

            let activated = continuation.clone();
            toast
                .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
                    move |_toast, args| {
                        if let Some(key) = activation_key(&args) {
                            dispatch_signal(
                                &NotificationSignal::ActionInvoked(key),
                                &activated,
                                &DismissedToast,
                            );
                        }
                        Ok(())
                    },
                ))
                .context("failed to observe WinRT toast activation")?;

            let dismissed = continuation.clone();
            toast
                .Dismissed(&TypedEventHandler::<
                    ToastNotification,
                    ToastDismissedEventArgs,
                >::new(move |_toast, args| {
                    let reason = args
                        .as_ref()
                        .and_then(|args| args.Reason().ok())
                        .map_or(4, freedesktop_reason);
                    dispatch_signal(
                        &NotificationSignal::Closed(reason),
                        &dismissed,
                        &DismissedToast,
                    );
                    Ok(())
                }))
                .context("failed to observe WinRT toast dismissal")?;

            self.notifier
                .Show(&toast)
                .context("failed to show the actionable WinRT toast")?;
            self.shown
                .lock()
                .map_err(|_| anyhow!("the WinRT toast handle was poisoned"))?
                .replace(toast);
            // WinRT has no numeric notification id. The continuation only ever
            // uses it to hide the toast it showed, and this sink holds exactly
            // one, so a constant is enough to satisfy the shared seam.
            Ok(NotificationId(0))
        }
    }

    impl super::ActionableSink for WinRtSink {
        fn show_and_watch(
            &self,
            actions: &NotificationActions,
            continuation: &ReadyCheckContinuation,
        ) -> Result<()> {
            let notification = self
                .notification
                .as_ref()
                .ok_or_else(|| anyhow!("the WinRT ready-check sink has no notification to show"))?;
            let content = toast_content(notification)?;
            let id = WinRtSink::show_and_watch(self, &content, actions, continuation)?;
            // The continuation only hides what it knows it showed, and this
            // path never went through `NotificationSink::show`.
            continuation.note_shown(id);
            Ok(())
        }
    }

    impl NotificationSink for WinRtSink {
        fn show(&self, _actions: &NotificationActions) -> Result<NotificationId> {
            Err(anyhow!(
                "an actionable WinRT toast is shown through show_and_watch"
            ))
        }

        fn hide(&self, _id: NotificationId) -> Result<()> {
            let toast = self
                .shown
                .lock()
                .map_err(|_| anyhow!("the WinRT toast handle was poisoned"))?
                .take();
            toast
                .map(|toast| {
                    self.notifier
                        .Hide(&toast)
                        .context("failed to hide the WinRT toast")
                })
                .transpose()?;
            Ok(())
        }
    }

    /// Reads the action key off an activation, or `None` when the toast was
    /// activated in a way that carries no arguments.
    fn activation_key(args: &Ref<'_, IInspectable>) -> Option<String> {
        args.as_ref()
            .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
            .and_then(|args| args.Arguments().ok())
            .map(|arguments| arguments.to_string_lossy())
    }

    /// Adds the Yes/No buttons and makes a body click report the default key.
    ///
    /// The `ToastText02` template carries no `<actions>` element, so it is
    /// built here. `launch` is what a body click reports, and setting it to the
    /// reserved default key is what lets both platforms share
    /// `activation_for_action_key`.
    fn attach_actions(content: &XmlDocument, actions: &NotificationActions) -> Result<()> {
        let toast = content
            .DocumentElement()
            .context("the WinRT toast template had no root element")?;
        toast
            .SetAttribute(&HSTRING::from("launch"), &HSTRING::from(DEFAULT_ACTION_KEY))
            .context("failed to set the WinRT toast launch argument")?;

        let actions_element = content
            .CreateElement(&HSTRING::from("actions"))
            .context("failed to create the WinRT toast actions element")?;
        for (key, label) in [
            (YES_ACTION_KEY, actions.yes.as_str()),
            (NO_ACTION_KEY, actions.no.as_str()),
        ] {
            let action = build_action(content, key, label)?;
            actions_element
                .AppendChild(&action)
                .context("failed to add a WinRT toast action")?;
        }
        toast
            .AppendChild(&actions_element)
            .context("failed to add the WinRT toast actions")?;
        Ok(())
    }

    fn build_action(content: &XmlDocument, key: &str, label: &str) -> Result<XmlElement> {
        let action = content
            .CreateElement(&HSTRING::from("action"))
            .context("failed to create a WinRT toast action")?;
        // `content` is the button text and `arguments` the key the activation
        // reports back. Swapping them shows the key as the button label.
        action
            .SetAttribute(&HSTRING::from("content"), &HSTRING::from(label))
            .context("failed to label a WinRT toast action")?;
        action
            .SetAttribute(&HSTRING::from("arguments"), &HSTRING::from(key))
            .context("failed to key a WinRT toast action")?;
        action
            .SetAttribute(
                &HSTRING::from("activationType"),
                &HSTRING::from("foreground"),
            )
            .context("failed to set a WinRT toast activation type")?;
        Ok(action)
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::ready_check_notification::{
        NotificationAction, NotificationActions, NotificationActivation, NotificationId,
        NotificationSink, ReadyCheckContinuation, ReadyCheckOutcome,
    };

    /// A backend whose toast presses Yes from the watcher's own thread, the
    /// way a freedesktop listener or a WinRT handler would.
    #[derive(Default)]
    struct AnsweringSink {
        hidden: Mutex<Vec<NotificationId>>,
    }

    impl NotificationSink for AnsweringSink {
        fn show(&self, _actions: &NotificationActions) -> anyhow::Result<NotificationId> {
            Ok(NotificationId(7))
        }

        fn hide(&self, id: NotificationId) -> anyhow::Result<()> {
            self.hidden.lock().expect("answering sink").push(id);
            Ok(())
        }
    }

    impl ActionableSink for AnsweringSink {
        fn show_and_watch(
            &self,
            actions: &NotificationActions,
            continuation: &ReadyCheckContinuation,
        ) -> anyhow::Result<()> {
            continuation.show(self, actions);
            continuation.activate(
                NotificationActivation::Chosen(NotificationAction::Yes),
                self,
            );
            Ok(())
        }
    }

    fn actions() -> NotificationActions {
        NotificationActions {
            yes: "Yes".to_owned(),
            no: "No".to_owned(),
        }
    }

    /// clonk-org/clonk-rs#1308: the lobby used to call `continuation.show`
    /// on a silent sink and never spawned the listener, so a real toast's
    /// buttons resolved nothing. The watcher thread owns both halves: it
    /// shows through the backend and routes the answer into the shared
    /// claim, hiding the toast it showed.
    #[test]
    fn a_watcher_thread_shows_the_toast_and_resolves_the_continuation() {
        let sink: Arc<dyn ActionableSink> = Arc::new(AnsweringSink::default());
        let continuation = ReadyCheckContinuation::new();
        let watcher = watch_on_thread(Arc::clone(&sink), actions(), continuation.clone())
            .expect("the watcher thread spawns");
        watcher.join().expect("the watcher finishes");
        assert_eq!(
            continuation.outcome(),
            Some(ReadyCheckOutcome::Answered(true))
        );
        assert!(
            continuation.shown_id().is_none(),
            "the answer hides the toast the watcher showed"
        );
    }

    /// The app hides through its own handle when the in-window dialog wins,
    /// so the sink the watcher shows through must be the same object the app
    /// keeps: an `Arc<dyn ActionableSink>` upcasts to the app's sink type.
    #[test]
    fn the_actionable_sink_is_the_app_sink() {
        let sink: Arc<dyn ActionableSink> = Arc::new(AnsweringSink::default());
        let app_sink: Arc<dyn NotificationSink + Send + Sync> = sink.clone();
        let continuation = ReadyCheckContinuation::new();
        continuation.show(app_sink.as_ref(), &actions());
        assert!(continuation.answer(false, app_sink.as_ref()));
        assert!(continuation.shown_id().is_none());
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    #[test]
    fn this_platform_offers_no_actionable_sink() {
        let notification = crate::desktop_notification::DesktopNotification::new(
            "Are you ready?",
            "body",
            std::time::Duration::from_secs(10),
        );
        assert!(platform_sink(&notification).is_none());
    }
}
