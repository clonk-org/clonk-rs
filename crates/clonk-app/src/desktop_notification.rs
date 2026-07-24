use std::time::Duration;

use anyhow::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopNotification {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) expiration: Duration,
}

impl DesktopNotification {
    pub(crate) fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        expiration: Duration,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            expiration,
        }
    }
}

pub(crate) struct DesktopNotifier {
    #[cfg(any(target_os = "linux", windows))]
    backend: backend::Notifier,
}

impl DesktopNotifier {
    pub(crate) fn initialize() -> Result<Option<Self>> {
        #[cfg(any(target_os = "linux", windows))]
        {
            return backend::Notifier::initialize().map(|backend| Some(Self { backend }));
        }

        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Ok(None)
        }
    }

    pub(crate) fn show(&self, notification: &DesktopNotification) -> Result<()> {
        #[cfg(any(target_os = "linux", windows))]
        {
            return self.backend.show(notification);
        }

        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = notification;
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use std::collections::HashMap;

    use anyhow::{Context, Result};
    use zbus::{
        blocking::{Connection, Proxy},
        zvariant::Value,
    };

    use super::DesktopNotification;

    pub(super) struct Notifier {
        connection: Connection,
    }

    impl Notifier {
        pub(super) fn initialize() -> Result<Self> {
            let connection = Connection::session()
                .context("failed to connect to the desktop notification session bus")?;
            Ok(Self { connection })
        }

        pub(super) fn show(&self, notification: &DesktopNotification) -> Result<()> {
            let proxy = Proxy::new(
                &self.connection,
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
            )
            .context("failed to create the desktop notification proxy")?;
            let actions = Vec::<&str>::new();
            let hints = HashMap::<&str, Value<'_>>::new();
            let expiration = i32::try_from(notification.expiration.as_millis()).unwrap_or(i32::MAX);
            let _: u32 = proxy
                .call(
                    "Notify",
                    &(
                        "Clonk Rust",
                        0_u32,
                        "",
                        notification.title.as_str(),
                        notification.body.as_str(),
                        actions,
                        hints,
                        expiration,
                    ),
                )
                .context("desktop notification service rejected the ready check")?;
            Ok(())
        }
    }
}

#[cfg(windows)]
mod backend {
    use std::mem::size_of;
    use std::slice;
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::{Context, Result};
    use windows::{
        core::{w, Interface, HSTRING},
        Data::Xml::Dom::XmlDocument,
        Foundation::{DateTime, IReference, PropertyValue},
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            System::{
                Registry::{
                    RegCloseKey, RegCreateKeyW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_SZ,
                },
                WinRT::{RoInitialize, RoUninitialize, RO_INIT_SINGLETHREADED},
            },
            UI::Shell::SetCurrentProcessExplicitAppUserModelID,
        },
        UI::Notifications::{
            ToastNotification, ToastNotificationManager, ToastNotifier, ToastTemplateType,
        },
    };

    use super::DesktopNotification;

    // Keep the established opaque notification identity so existing Windows
    // toast permissions and history continue to apply after the visible rename.
    const APP_USER_MODEL_ID: &str = "LegacyClonkTeam.LegacyClonk";

    pub(super) struct Notifier {
        notifier: Option<ToastNotifier>,
        apartment_initialized: bool,
    }

    impl Notifier {
        pub(super) fn initialize() -> Result<Self> {
            let apartment_initialized = match unsafe { RoInitialize(RO_INIT_SINGLETHREADED) } {
                Ok(()) => true,
                Err(error) if error.code() == RPC_E_CHANGED_MODE => false,
                Err(error) => {
                    return Err(error).context("failed to initialize the WinRT apartment");
                }
            };
            let result = (|| {
                if let Err(error) = unsafe {
                    SetCurrentProcessExplicitAppUserModelID(w!("LegacyClonkTeam.LegacyClonk"))
                } {
                    tracing::warn!(%error, "failed to set the Clonk Rust application identity");
                }
                if let Err(error) = register_app_user_model_id() {
                    tracing::warn!(%error, "failed to register the Clonk Rust application identity");
                }
                ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
                    APP_USER_MODEL_ID,
                ))
                .context("failed to initialize the WinRT toast notifier")
            })();
            match result {
                Ok(notifier) => Ok(Self {
                    notifier: Some(notifier),
                    apartment_initialized,
                }),
                Err(error) => {
                    if apartment_initialized {
                        unsafe { RoUninitialize() };
                    }
                    Err(error)
                }
            }
        }

        pub(super) fn show(&self, notification: &DesktopNotification) -> Result<()> {
            let content =
                ToastNotificationManager::GetTemplateContent(ToastTemplateType::ToastText02)
                    .context("failed to create WinRT toast content")?;
            set_text_node(&content, 0, &notification.title)?;
            set_text_node(&content, 1, &notification.body)?;
            let toast = ToastNotification::CreateToastNotification(&content)
                .context("failed to create WinRT toast")?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let expiration = now.saturating_add(notification.expiration);
            let ticks = i128::from(expiration.as_secs()) * 10_000_000
                + i128::from(expiration.subsec_nanos() / 100)
                + 116_444_736_000_000_000_i128;
            let boxed = PropertyValue::CreateDateTime(DateTime {
                UniversalTime: i64::try_from(ticks).unwrap_or(i64::MAX),
            })?
            .cast::<IReference<DateTime>>()?;
            toast
                .SetExpirationTime(&boxed)
                .context("failed to set WinRT toast expiration")?;
            self.notifier
                .as_ref()
                .expect("notifier exists until drop")
                .Show(&toast)
                .context("failed to show WinRT toast")?;
            Ok(())
        }
    }

    impl Drop for Notifier {
        fn drop(&mut self) {
            self.notifier.take();
            if self.apartment_initialized {
                unsafe { RoUninitialize() };
            }
        }
    }

    fn set_text_node(content: &XmlDocument, index: u32, text: &str) -> Result<()> {
        content
            .GetElementsByTagName(&HSTRING::from("text"))
            .context("failed to find WinRT toast text nodes")?
            .Item(index)
            .context("WinRT toast template omitted a text node")?
            .SetInnerText(&HSTRING::from(text))
            .context("failed to populate WinRT toast text")?;
        Ok(())
    }

    struct RegistryKey(HKEY);

    impl Drop for RegistryKey {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn register_app_user_model_id() -> Result<()> {
        let mut key = HKEY::default();
        unsafe {
            RegCreateKeyW(
                HKEY_CURRENT_USER,
                w!("Software\\Classes\\AppUserModelId\\LegacyClonkTeam.LegacyClonk"),
                &mut key,
            )
            .ok()
        }
        .context("failed to create the AppUserModelId registry key")?;
        let key = RegistryKey(key);
        let display_name = "Clonk Rust\0".encode_utf16().collect::<Vec<_>>();
        let display_name_bytes = unsafe {
            slice::from_raw_parts(
                display_name.as_ptr().cast::<u8>(),
                display_name.len() * size_of::<u16>(),
            )
        };
        unsafe {
            RegSetValueExW(
                key.0,
                w!("DisplayName"),
                0,
                REG_SZ,
                Some(display_name_bytes),
            )
            .ok()
        }
        .context("failed to set the AppUserModelId display name")?;
        Ok(())
    }
}
