//! Platform backends for the actionable ready-check notification.
//!
//! The platform-independent core — the atomic claim, the action-key mapping
//! and the signal routing — lives in [`crate::ready_check_notification`]. Only
//! the translation between one platform's notification API and that core lives
//! here, so the race the core owns is never re-implemented per platform.

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
        shown: std::sync::Mutex<Option<ToastNotification>>,
    }

    impl WinRtSink {
        pub(crate) fn new(notifier: ToastNotifier) -> Self {
            Self {
                notifier,
                shown: std::sync::Mutex::new(None),
            }
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
