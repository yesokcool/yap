//! Desktop notifications for @mentions, pings and join requests.
//!
//! * Windows: a WinRT toast under our own AppUserModelID ("Yap"), registered
//!   per user in HKCU (how unpackaged apps get toasts with their own name).
//! * Linux: `notify-send` (libnotify; on every mainstream desktop).
//! * macOS: `osascript`'s `display notification`.
//!
//! Always fire-and-forget on a short-lived thread; failures are only logged.

pub fn show(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.chars().take(240).collect::<String>());
    let _ = std::thread::Builder::new().name("yap-notify".into()).spawn(move || {
        if let Err(e) = platform::show(&title, &body) {
            log::debug!("notification failed: {e}");
        }
    });
}

/// The Dock icon's badge: how many mentions are unread (none at 0). macOS
/// only, and on the main thread (the UI calls it from its frame).
#[cfg(target_os = "macos")]
pub fn dock_badge(count: u32) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    let Some(cls) = AnyClass::get(c"NSApplication") else { return };
    let label = (count > 0).then(|| NSString::from_str(&if count > 99 { "99+".to_string() } else { count.to_string() }));
    // SAFETY: plain AppKit messages to the shared application and its Dock
    // tile, on the main thread; a missing one is checked, not messaged.
    unsafe {
        let app: *mut AnyObject = msg_send![cls, sharedApplication];
        let Some(app) = app.as_ref() else { return };
        let tile: *mut AnyObject = msg_send![app, dockTile];
        let Some(tile) = tile.as_ref() else { return };
        let _: () = msg_send![tile, setBadgeLabel: label.as_deref()];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn dock_badge(_count: u32) {}

#[cfg(target_os = "windows")]
mod platform {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::{HSTRING, PCWSTR};

    const AUMID: &str = "Yap.Desktop";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// HKCU\Software\Classes\AppUserModelId\Yap.Desktop { DisplayName = "Yap" }.
    fn register() {
        let path = wide(&format!("Software\\Classes\\AppUserModelId\\{AUMID}"));
        let mut key = HKEY::default();
        // SAFETY: plain registry calls with valid, NUL-terminated UTF-16 buffers.
        unsafe {
            if RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None).is_ok() {
                let value = wide("Yap");
                let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
                let name = wide("DisplayName");
                let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
                let _ = RegCloseKey(key);
            }
        }
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    pub fn show(title: &str, body: &str) -> Result<(), String> {
        // SAFETY: initializing WinRT on this short-lived worker thread.
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        register();
        let xml = format!(
            r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio silent="true"/></toast>"#,
            escape(title),
            escape(body)
        );
        let doc = XmlDocument::new().map_err(|e| e.to_string())?;
        doc.LoadXml(&HSTRING::from(xml)).map_err(|e| e.to_string())?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)).map_err(|e| e.to_string())?;
        notifier.Show(&toast).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    pub fn show(title: &str, body: &str) -> Result<(), String> {
        std::process::Command::new("notify-send")
            // "--": a name or message starting with "-" is text, not an option (notify-send refused it).
            .args(["--app-name=Yap", "--icon=user-available", "--", title, body])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    fn quote(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }

    pub fn show(title: &str, body: &str) -> Result<(), String> {
        let script = format!("display notification {} with title {}", quote(body), quote(title));
        std::process::Command::new("osascript").args(["-e", &script]).status().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod platform {
    pub fn show(_title: &str, _body: &str) -> Result<(), String> {
        Ok(())
    }
}
