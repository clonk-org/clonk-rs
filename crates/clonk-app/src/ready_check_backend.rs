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

        /// Reads signals until the toast being watched is gone.
        ///
        /// Blocking, so a caller runs it on its own thread. It returns when
        /// [`dispatch_signal`] reports the toast no longer exists — which
        /// includes the `NotificationClosed` that our own `CloseNotification`
        /// produces. That is what makes an *unanswered* prompt's listener exit
        /// when the lobby tears the continuation down, rather than blocking on
        /// a bus that will never mention it again.
        pub(crate) fn listen(
            &self,
            id: NotificationId,
            continuation: &ReadyCheckContinuation,
        ) -> Result<()> {
            let signals = self
                .proxy()?
                .receive_all_signals()
                .context("failed to subscribe to desktop notification signals")?;
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
