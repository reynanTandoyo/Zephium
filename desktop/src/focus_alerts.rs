//! A focus phase that ends while another app is in front says so through the
//! system's notifications; in front, chrome's own notice is enough.

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::atomic::{AtomicBool, Ordering};

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationSound, UNUserNotificationCenter,
    };

    static ASKED: AtomicBool = AtomicBool::new(false);

    /// The notification center exists only for a bundled app; an unbundled
    /// development binary raises on first touch instead of returning nothing.
    fn bundled() -> bool {
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
    }

    /// Asked once, when the first session starts, so the prompt arrives with
    /// the action it is for rather than at launch.
    pub fn ask() {
        if !bundled() || ASKED.swap(true, Ordering::AcqRel) {
            return;
        }
        let done = RcBlock::new(|_granted: Bool, _error: *mut NSError| {});
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &done,
            );
    }

    /// One identifier per subject, so a newer alert replaces the last
    /// instead of stacking a column of stale ones.
    pub fn post(identifier: &str, title: &str, body: &str) {
        if !bundled() {
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(identifier),
            &content,
            None,
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, None);
    }
}

/// The session started: the moment to ask for permission to notify.
pub fn session_started() {
    ask_to_notify();
}

/// Asks once for permission to notify, when the first alert is near.
pub(crate) fn ask_to_notify() {
    #[cfg(target_os = "macos")]
    macos::ask();
}

/// Posts a system notification; a later one with the same identifier replaces it.
pub(crate) fn notify(identifier: &str, title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    macos::post(identifier, title, body);
    #[cfg(target_os = "windows")]
    windows::post(identifier, title, body);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (identifier, title, body);
}

/// Takes Zephium's taskbar and notification identity; called once at launch.
pub(crate) fn claim_identity(identifier: &str) {
    #[cfg(target_os = "windows")]
    windows::claim(identifier);
    #[cfg(not(target_os = "windows"))]
    let _ = identifier;
}

#[cfg(target_os = "windows")]
mod windows {
    use std::sync::OnceLock;

    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::{
        FOLDERID_AppsFolder, IShellItem, SHCreateItemInKnownFolder,
        SetCurrentProcessExplicitAppUserModelID, KF_FLAG_DEFAULT,
    };
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    static APP_ID: OnceLock<HSTRING> = OnceLock::new();

    /// Takes the identity the installer gives Zephium's shortcuts, so the
    /// taskbar groups with a pinned one and a toast is shown under its name.
    pub fn claim(identifier: &str) {
        let id = HSTRING::from(identifier);
        // SAFETY: a process-wide setting, made before any window exists.
        if unsafe { SetCurrentProcessExplicitAppUserModelID(&id) }.is_ok() {
            let _ = APP_ID.set(id);
        }
    }

    /// Windows shows a toast for any app ID, even one nothing installed
    /// carries. Only an ID an installed shortcut gives Start is used, so a
    /// development or performance build posts none.
    fn installed(app: &HSTRING) -> bool {
        // SAFETY: a lookup by name in the Apps folder on this thread's own
        // apartment; the item is released when dropped.
        unsafe {
            SHCreateItemInKnownFolder::<_, IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, app)
        }
        .is_ok()
    }

    pub fn post(identifier: &str, title: &str, body: &str) {
        let Some(app) = APP_ID.get() else {
            return;
        };
        let (tag, xml) = (super::toast_tag(identifier), super::toast_xml(title, body));
        let show = move || -> windows::core::Result<()> {
            if !installed(app) {
                return Ok(());
            }
            let document = XmlDocument::new()?;
            document.LoadXml(&HSTRING::from(xml))?;
            let toast = ToastNotification::CreateToastNotification(&document)?;
            toast.SetTag(&HSTRING::from(tag))?;
            ToastNotificationManager::CreateToastNotifierWithId(app)?.Show(&toast)
        };
        // The Apps folder lookup needs COM, which the caller's thread may not
        // have; a short-lived thread owns its own apartment.
        let spawned = std::thread::Builder::new()
            .name("toast".into())
            .spawn(move || {
                // SAFETY: balanced below on this same thread.
                let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                if let Err(error) = show() {
                    eprintln!("notifications: the toast was not shown: {error}");
                }
                if joined {
                    // SAFETY: pairs the successful CoInitializeEx above.
                    unsafe { CoUninitialize() };
                }
            });
        if spawned.is_err() {
            eprintln!("notifications: the toast was not shown");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_an_app_id_start_knows_may_post() {
            // SAFETY: balanced below on this test's thread.
            let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            assert!(installed(&HSTRING::from("Microsoft.Windows.Explorer")));
            assert!(!installed(&HSTRING::from("app.zephium.never-installed")));
            if joined {
                // SAFETY: pairs the CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        }
    }
}

/// Text only, escaped, with no image or link a toast could fetch.
#[cfg(any(test, target_os = "windows"))]
fn toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_text(title),
        xml_text(body)
    )
}

#[cfg(any(test, target_os = "windows"))]
fn xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            // A character XML cannot carry would fail the whole toast.
            c if c.is_control() || matches!(c, '\u{FFFE}' | '\u{FFFF}') => {}
            c => out.push(c),
        }
    }
    out
}

/// A toast tag holds at most 64 characters; one subject keeps one tag, so
/// its newer toast replaces the last.
#[cfg(any(test, target_os = "windows"))]
fn toast_tag(identifier: &str) -> String {
    let hash = identifier
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{hash:016x}")
}

/// One of Zephium's windows is in front, so chrome's own notice is enough.
pub(crate) fn in_front(app: &tauri::AppHandle) -> bool {
    use tauri::Manager;
    [crate::MAIN_LABEL, crate::overlay::PANEL_LABEL]
        .iter()
        .any(|label| {
            app.get_webview_window(label)
                .is_some_and(|window| window.is_focused().unwrap_or(false))
        })
}

/// `alert` is the shell's `focus.alert=` value.
pub fn phase_ended(app: &tauri::AppHandle, alert: &str) {
    if in_front(app) {
        return;
    }
    let (title, body) = match alert {
        "finished" => (
            "Focus complete",
            "Your round is done. Shut sites are open again.",
        ),
        "break" => (
            "Time for a break",
            "Shut sites are open until the next round.",
        ),
        "focus" => ("Back to focus", "A new round has started."),
        _ => return,
    };
    notify("app.zephium.focus", title, body);
}

/// One identifier for every mail batch: a newer batch replaces the last
/// toast rather than stacking a column of counts.
const MAIL_ALERT: &str = "app.zephium.mail";
/// Longest title or body a mail toast may carry; a count sentence is far shorter.
const MAIL_TEXT_MAX: usize = 120;

/// A toast for a batch of new mail. The frame counts the batch and words it
/// in the person's language; this only checks the text is a short line. No
/// sender, subject or message body ever reaches this function.
#[tauri::command]
#[specta::specta]
pub(crate) fn mail_notify_new(caller: tauri::WebviewWindow, title: String, body: String) {
    if !super::authorize(&caller, super::CallerPolicy::Main, "mail_notify_new") {
        return;
    }
    let Some((title, body)) = mail_toast(&title, &body) else {
        return;
    };
    ask_to_notify();
    notify(MAIL_ALERT, &title, &body);
}

/// The toast a mail batch shows, or nothing when the text is empty, too long
/// or carries a line break a single-line toast cannot show.
fn mail_toast(title: &str, body: &str) -> Option<(String, String)> {
    let line = |text: &str| {
        // A carriage return or line feed anywhere is refused, not trimmed away.
        let fits = !text.contains(['\n', '\r']) && text.chars().count() <= MAIL_TEXT_MAX;
        let text = text.trim();
        (fits && !text.is_empty()).then(|| text.to_owned())
    };
    Some((line(title)?, line(body)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mail_batch_is_one_short_count_line_and_nothing_else() {
        assert_eq!(
            mail_toast("New mail", "3 new messages"),
            Some(("New mail".into(), "3 new messages".into()))
        );
        // One toast per batch: every batch shares the one identifier.
        assert_eq!(toast_tag(MAIL_ALERT), toast_tag(MAIL_ALERT));
        assert_ne!(toast_tag(MAIL_ALERT), toast_tag("app.zephium.focus"));
    }

    #[test]
    fn empty_long_or_multiline_mail_text_posts_nothing() {
        assert_eq!(mail_toast("", "1 new message"), None);
        assert_eq!(mail_toast("New mail", "   "), None);
        assert_eq!(mail_toast("New mail", "1 new message\nFrom: someone"), None);
        assert_eq!(mail_toast("New mail\r", "1 new message"), None);
        let long = "n".repeat(MAIL_TEXT_MAX + 1);
        assert_eq!(mail_toast("New mail", &long), None);
        let exact = "n".repeat(MAIL_TEXT_MAX);
        assert_eq!(
            mail_toast("New mail", &exact),
            Some(("New mail".into(), exact))
        );
    }

    #[test]
    fn a_mail_toast_is_escaped_text_only() {
        let (title, body) = mail_toast("New mail", "2 new messages <b>&</b>").expect("short line");
        assert_eq!(
            toast_xml(&title, &body),
            concat!(
                "<toast><visual><binding template=\"ToastGeneric\">",
                "<text>New mail</text>",
                "<text>2 new messages &lt;b&gt;&amp;&lt;/b&gt;</text>",
                "</binding></visual></toast>"
            )
        );
    }

    #[test]
    fn toast_text_is_escaped_and_nothing_else_is_added() {
        assert_eq!(
            toast_xml(
                "Work needs you",
                "Sign in to <img src=\"http://x\"/>&'y'\u{0}\u{7} now"
            ),
            concat!(
                "<toast><visual><binding template=\"ToastGeneric\">",
                "<text>Work needs you</text>",
                "<text>Sign in to &lt;img src=&quot;http://x&quot;/&gt;&amp;&apos;y&apos; now</text>",
                "</binding></visual></toast>"
            )
        );
    }

    #[test]
    fn one_subject_keeps_one_short_tag() {
        let long = format!("app.zephium.work.{}", "k".repeat(200));
        assert_eq!(toast_tag(&long), toast_tag(&long.clone()));
        assert_ne!(toast_tag("app.zephium.focus"), toast_tag(&long));
        assert!(toast_tag(&long).len() <= 64);
    }
}
