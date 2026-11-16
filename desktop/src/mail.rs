//! Fork-owned (T-0020 to T-0032): the mail commands and their one event.
//!
//! The frame projects state and submits typed intent. Passwords cross this
//! boundary once, inbound, and go straight to the OS vault; nothing here ever
//! returns, stores or logs one. Server text never reaches the frame: every
//! failure is the closed [`MailErrorV1`].
use super::*;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use zephium_mail::auth::{AuthKind, Credential};
use zephium_mail::autoconfig::{self, Autoconfig, AutoconfigSource, ConfigFetcher};
use zephium_mail::engine::MailEngine;
use zephium_mail::limits::{MAX_ACCOUNTS, MAX_AUTOCONFIG_BYTES, MAX_PAGE};
use zephium_mail::model::{
    self as mail, Account, Address, AttachmentInfo, Draft, Endpoint, FlagKind, FolderRole,
    MessageBody, MessageHeader, OutgoingAttachment, Security, WatchOutcome,
};
use zephium_mail::remote_images::{self, FetchedImage, ImageFetcher};
use zephium_mail::rules::{self, Rule, RuleAction, RuleField};
use zephium_mail::smtp::LettreSender;
use zephium_mail::transport::TlsConnectorImpl;
use zephium_mail::vault::{OsVault, PasswordVault};
use zephium_mail::{BoxFuture, MailError};
use zephium_store::mail_cache::{
    CacheResult, MailAccountRecord, MailBodyRecord, MailFolderRecord, MailMessageRecord,
};
use zeroize::Zeroize;

use crate::oauth::{self, OAuthProviderV1};

const EVENT: &str = "zephium:mail-changed";
const SETTING_ENABLED: &str = "mail.enabled";
/// JSON map of account id to [`AuthKind`] for OAuth accounts. Holds client
/// IDs and tenants only, never a token; absent means a password account.
const SETTING_AUTH: &str = "mail.auth";
/// JSON array of account ids in the order the person arranged them. Ids that
/// are missing from it (new accounts) follow in the order they were added;
/// ids that no longer exist are ignored.
const SETTING_ORDER: &str = "mail.account_order";
/// JSON map of account id to that account's ordered rules (T-0036). Rules see
/// only their own account's Inbox headers. Bounded by [`rules::MAX_RULES_BYTES`].
const SETTING_RULES: &str = "mail.rules";
const EVENT_COMPOSE: &str = "zephium:mail-compose";
const MAX_IN_FLIGHT: usize = 8;
const MAX_DRAFT_BYTES: usize = 48 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Why a mail command failed. A closed set: no server text.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailErrorV1 {
    InvalidInput,
    NotFound,
    Offline,
    Timeout,
    Tls,
    AuthFailed,
    ServerRejected,
    Protocol,
    TooLarge,
    Credentials,
    Unsupported,
    Busy,
    Internal,
    /// The outgoing server refused a recipient or the sender address.
    AddressRejected,
    /// No Trash folder: a delete needs a confirmed `permanent` flag.
    NoTrash,
    /// Mail is switched off in Settings.
    Disabled,
}

impl From<MailError> for MailErrorV1 {
    fn from(error: MailError) -> Self {
        match error {
            MailError::InvalidInput => Self::InvalidInput,
            MailError::NotFound => Self::NotFound,
            MailError::Offline => Self::Offline,
            MailError::Timeout => Self::Timeout,
            MailError::Tls => Self::Tls,
            MailError::AuthFailed => Self::AuthFailed,
            MailError::ServerRejected => Self::ServerRejected,
            MailError::Protocol => Self::Protocol,
            MailError::TooLarge => Self::TooLarge,
            MailError::Credentials => Self::Credentials,
            MailError::Unsupported => Self::Unsupported,
            MailError::Busy => Self::Busy,
            MailError::Internal => Self::Internal,
            MailError::AddressRejected => Self::AddressRejected,
            MailError::NoTrash => Self::NoTrash,
        }
    }
}

type Reply<T> = Result<T, MailErrorV1>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailSecurityV1 {
    Tls,
    StartTls,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailEndpointV1 {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) security: MailSecurityV1,
}

/// An account as the frame sees it. There is no password field.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailAccountV1 {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) email: String,
    pub(crate) imap: MailEndpointV1,
    pub(crate) smtp: MailEndpointV1,
    pub(crate) username: String,
    pub(crate) colour: String,
    pub(crate) signature: String,
    pub(crate) save_sent_copy: bool,
    /// Set only for OAuth accounts: who they sign in with. `null` for
    /// password accounts.
    #[serde(default)]
    #[specta(optional)]
    pub(crate) oauth: Option<MailOAuthV1>,
    /// The server refused the sign-in while watching the inbox: the password
    /// or token needs attention. Cleared by updating the account or refreshing.
    #[serde(default)]
    pub(crate) sign_in_failed: bool,
}

/// The OAuth identity of an account. No token.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailOAuthV1 {
    pub(crate) provider: OAuthProviderV1,
    pub(crate) client_id: String,
    pub(crate) tenant: Option<String>,
}

/// What the person typed to add or test an account. The password is read
/// once, written to the OS vault and wiped; this type is not `Debug` so it
/// cannot be formatted into a log by accident.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub(crate) struct MailNewAccountV1 {
    pub(crate) display_name: String,
    pub(crate) email: String,
    pub(crate) imap: MailEndpointV1,
    pub(crate) smtp: MailEndpointV1,
    pub(crate) username: String,
    pub(crate) colour: String,
    pub(crate) signature: String,
    pub(crate) save_sent_copy: bool,
    /// Empty when `oauth_grant` is given.
    pub(crate) password: String,
    /// A handle from `oauth_sign_in`: signs the account in with OAuth
    /// instead of a password. Consumed by `mail_add_account`.
    #[serde(default)]
    #[specta(optional)]
    pub(crate) oauth_grant: Option<String>,
}

impl Drop for MailNewAccountV1 {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

/// Changes to an existing account. `password` is `Some` only to replace it.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub(crate) struct MailAccountUpdateV1 {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) email: String,
    pub(crate) imap: MailEndpointV1,
    pub(crate) smtp: MailEndpointV1,
    pub(crate) username: String,
    pub(crate) colour: String,
    pub(crate) signature: String,
    pub(crate) save_sent_copy: bool,
    pub(crate) password: Option<String>,
    /// A handle from `oauth_sign_in` to sign an OAuth account in again.
    #[serde(default)]
    #[specta(optional)]
    pub(crate) oauth_grant: Option<String>,
}

impl Drop for MailAccountUpdateV1 {
    fn drop(&mut self) {
        if let Some(password) = &mut self.password {
            password.zeroize();
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailAutoconfigSourceV1 {
    Ispdb,
    ProviderSubdomain,
    WellKnown,
}

/// The result of provider autoconfig. `found: false` means enter it by hand.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailAutoconfigV1 {
    pub(crate) found: bool,
    pub(crate) imap: Option<MailEndpointV1>,
    pub(crate) smtp: Option<MailEndpointV1>,
    pub(crate) username: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) source: Option<MailAutoconfigSourceV1>,
}

/// Each server's outcome when testing an account; `None` means it worked.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailTestV1 {
    pub(crate) imap: Option<MailErrorV1>,
    pub(crate) smtp: Option<MailErrorV1>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailFolderRoleV1 {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Archive,
    Junk,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailFolderV1 {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) delimiter: Option<String>,
    pub(crate) role: MailFolderRoleV1,
    pub(crate) unread: u32,
    pub(crate) total: u32,
    pub(crate) selectable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailAddressV1 {
    pub(crate) name: Option<String>,
    pub(crate) email: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailFlagsV1 {
    pub(crate) seen: bool,
    pub(crate) flagged: bool,
    pub(crate) answered: bool,
    pub(crate) draft: bool,
    pub(crate) deleted: bool,
}

/// One row of the message list.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, PartialEq)]
pub(crate) struct MailMessageV1 {
    pub(crate) uid: u32,
    pub(crate) message_id: Option<String>,
    pub(crate) subject: String,
    pub(crate) from: Vec<MailAddressV1>,
    pub(crate) to: Vec<MailAddressV1>,
    pub(crate) cc: Vec<MailAddressV1>,
    /// Unix seconds.
    pub(crate) date: Option<f64>,
    pub(crate) flags: MailFlagsV1,
    pub(crate) size: u32,
    pub(crate) has_attachments: bool,
    pub(crate) snippet: String,
    pub(crate) in_reply_to: Option<String>,
    pub(crate) references: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailPageRequestV1 {
    /// Exclusive UID bound; `None` for the newest page.
    pub(crate) before_uid: Option<u32>,
    pub(crate) limit: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, PartialEq)]
pub(crate) struct MailPageV1 {
    pub(crate) messages: Vec<MailMessageV1>,
    /// Pass as `before_uid` for the next page; `None` at the end.
    pub(crate) next_before: Option<u32>,
    pub(crate) total: u32,
    /// True when the server was unreachable and this is the local cache.
    pub(crate) from_cache: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailAttachmentV1 {
    pub(crate) part: u32,
    pub(crate) filename: String,
    pub(crate) content_type: String,
    pub(crate) size: u32,
    pub(crate) content_id: Option<String>,
    pub(crate) inline: bool,
}

/// An inline image, base64-encoded, for rewriting `cid:` references.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailInlineImageV1 {
    pub(crate) content_id: String,
    pub(crate) content_type: String,
    pub(crate) data_base64: String,
}

/// A message with its body. `html` is raw and unsanitised: the frame must
/// sanitise it before display.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, PartialEq)]
pub(crate) struct MailBodyV1 {
    pub(crate) header: MailMessageV1,
    pub(crate) reply_to: Vec<MailAddressV1>,
    pub(crate) text: Option<String>,
    pub(crate) html: Option<String>,
    pub(crate) truncated: bool,
    pub(crate) attachments: Vec<MailAttachmentV1>,
    pub(crate) inline_images: Vec<MailInlineImageV1>,
    /// Calendar invitations in the message (`text/calendar` parts and `.ics`
    /// files), at most four. Read one with `mail_invitation_bytes`.
    pub(crate) invitations: Vec<MailInvitationPartV1>,
}

/// A calendar part of a message.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailInvitationPartV1 {
    /// The attachment index, as for `mail_attachment`.
    pub(crate) part: u32,
    /// The part's `method` parameter in capitals (`REQUEST`, `CANCEL`,
    /// `REPLY`...), when it names one.
    pub(crate) method: Option<String>,
    pub(crate) filename: String,
    pub(crate) size: u32,
}

/// The `text/calendar` alternative of a reply. Only `REPLY` is accepted.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailCalendarBodyV1 {
    pub(crate) method: String,
    pub(crate) ics: String,
}

/// A remote image the backend fetched for a message, as a `data:` URL. The
/// frame substitutes it for `url` in the sanitised HTML.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailRemoteImageV1 {
    /// The address exactly as the message wrote it.
    pub(crate) url: String,
    /// `data:image/{png,jpeg,gif,webp};base64,...`
    pub(crate) data_url: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailFlagV1 {
    Seen,
    Flagged,
    Deleted,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailOutgoingAttachmentV1 {
    pub(crate) filename: String,
    pub(crate) content_type: String,
    pub(crate) data_base64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailDraftV1 {
    pub(crate) to: Vec<MailAddressV1>,
    pub(crate) cc: Vec<MailAddressV1>,
    pub(crate) bcc: Vec<MailAddressV1>,
    pub(crate) subject: String,
    pub(crate) text: String,
    pub(crate) html: Option<String>,
    pub(crate) attachments: Vec<MailOutgoingAttachmentV1>,
    pub(crate) in_reply_to: Option<String>,
    pub(crate) references: Vec<String>,
    /// Makes the message an iMIP reply: `text` and this iCalendar text are
    /// sent as alternatives. Not allowed together with `html` or attachments.
    #[serde(default)]
    #[specta(optional)]
    pub(crate) calendar: Option<MailCalendarBodyV1>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailSendOutcomeV1 {
    /// The message went out. False never occurs: a failed send is an `Err`.
    pub(crate) sent: bool,
    /// The copy in Sent was also stored.
    pub(crate) saved_copy: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailSaveOutcomeV1 {
    Saved,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailChangeKindV1 {
    /// A folder gained messages (the inbox first).
    NewMail,
    /// Counts or flags moved.
    Counts,
    /// Messages were moved or deleted here.
    Removed,
    /// The account list changed.
    Accounts,
}

/// Something in mail changed; read it again. Carries counts only: no sender,
/// subject or address, so the event can never leak mail content.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[tauri_specta(event_name = "zephium:mail-changed")]
pub(crate) struct MailChanged {
    pub(crate) account: String,
    pub(crate) kind: MailChangeKindV1,
    pub(crate) folder: Option<String>,
    /// Unread messages in `folder` after the change.
    pub(crate) unread: u32,
    /// Messages that arrived, for `NewMail`.
    pub(crate) new_messages: u32,
}

/// An address suggested while writing, from cached mail only.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailContactV1 {
    pub(crate) name: Option<String>,
    pub(crate) email: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailRuleFieldV1 {
    From,
    To,
    Subject,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MailRuleActionV1 {
    Move,
    MarkRead,
    Flag,
}

/// "If `field` contains `contains`, then `action`." `folder` is the
/// destination of a move and `null` for the other actions.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailRuleV1 {
    pub(crate) field: MailRuleFieldV1,
    pub(crate) contains: String,
    pub(crate) action: MailRuleActionV1,
    #[serde(default)]
    #[specta(optional)]
    pub(crate) folder: Option<String>,
}

/// A parsed `mailto:` link: only the fields a compose window starts with.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct MailtoV1 {
    pub(crate) to: Vec<String>,
    pub(crate) cc: Vec<String>,
    pub(crate) bcc: Vec<String>,
    pub(crate) subject: String,
    pub(crate) body: String,
}

/// A `mailto:` link was clicked and Mail should open a prefilled message.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[tauri_specta(event_name = "zephium:mail-compose")]
pub(crate) struct MailComposeRequested {
    pub(crate) mailto: MailtoV1,
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn security_in(security: MailSecurityV1) -> Security {
    match security {
        MailSecurityV1::Tls => Security::Tls,
        MailSecurityV1::StartTls => Security::StartTls,
    }
}

fn security_out(security: Security) -> MailSecurityV1 {
    match security {
        Security::Tls => MailSecurityV1::Tls,
        Security::StartTls => MailSecurityV1::StartTls,
    }
}

fn security_text(security: Security) -> &'static str {
    match security {
        Security::Tls => "tls",
        Security::StartTls => "starttls",
    }
}

fn security_parse(text: &str) -> Option<Security> {
    match text {
        "tls" => Some(Security::Tls),
        "starttls" => Some(Security::StartTls),
        _ => None,
    }
}

fn endpoint_in(endpoint: &MailEndpointV1) -> Endpoint {
    Endpoint {
        host: endpoint.host.trim().to_ascii_lowercase(),
        port: endpoint.port,
        security: security_in(endpoint.security),
    }
}

fn endpoint_out(endpoint: &Endpoint) -> MailEndpointV1 {
    MailEndpointV1 {
        host: endpoint.host.clone(),
        port: endpoint.port,
        security: security_out(endpoint.security),
    }
}

fn account_out(account: &Account) -> MailAccountV1 {
    MailAccountV1 {
        id: account.id.clone(),
        display_name: account.display_name.clone(),
        email: account.email.clone(),
        imap: endpoint_out(&account.imap),
        smtp: endpoint_out(&account.smtp),
        username: account.username.clone(),
        colour: account.colour.clone(),
        signature: account.signature.clone(),
        save_sent_copy: account.save_sent_copy,
        sign_in_failed: is_sign_in_failed(&account.id),
        oauth: match &account.auth {
            AuthKind::Password => None,
            AuthKind::Oauth {
                provider,
                client_id,
                tenant,
            } => Some(MailOAuthV1 {
                provider: (*provider).into(),
                client_id: client_id.clone(),
                tenant: tenant.clone(),
            }),
        },
    }
}

fn account_record(account: &Account) -> MailAccountRecord {
    MailAccountRecord {
        id: account.id.clone(),
        display_name: account.display_name.clone(),
        email: account.email.clone(),
        imap_host: account.imap.host.clone(),
        imap_port: account.imap.port,
        imap_security: security_text(account.imap.security).to_owned(),
        smtp_host: account.smtp.host.clone(),
        smtp_port: account.smtp.port,
        smtp_security: security_text(account.smtp.security).to_owned(),
        username: account.username.clone(),
        colour: account.colour.clone(),
        signature: account.signature.clone(),
        save_sent_copy: account.save_sent_copy,
    }
}

/// Serializes the read-modify-write of the auth map.
static AUTH_LOCK: Mutex<()> = Mutex::new(());

fn auth_map() -> HashMap<String, AuthKind> {
    use zephium_core::ports::store::Store;
    APP_STORE
        .get()
        .and_then(|store| store.app_setting(SETTING_AUTH))
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// How an account signs in; anything unreadable is a password account.
fn auth_of(id: &str) -> AuthKind {
    auth_map().remove(id).unwrap_or_default()
}

/// Records (or, for `None`, forgets) an account's OAuth identity.
fn set_auth(id: &str, auth: Option<AuthKind>) -> bool {
    use zephium_core::ports::store::Store;
    let _guard = AUTH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = auth_map();
    match auth {
        Some(kind @ AuthKind::Oauth { .. }) => map.insert(id.to_owned(), kind),
        _ => map.remove(id),
    };
    let Ok(json) = serde_json::to_string(&map) else {
        return false;
    };
    APP_STORE
        .get()
        .is_some_and(|store| store.set_app_setting(SETTING_AUTH.to_owned(), json))
}

/// Serializes the read-modify-write of the order list.
static ORDER_LOCK: Mutex<()> = Mutex::new(());

fn account_order() -> Vec<String> {
    use zephium_core::ports::store::Store;
    APP_STORE
        .get()
        .and_then(|store| store.app_setting(SETTING_ORDER))
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn store_account_order(order: &[String]) -> bool {
    use zephium_core::ports::store::Store;
    let Ok(json) = serde_json::to_string(order) else {
        return false;
    };
    APP_STORE
        .get()
        .is_some_and(|store| store.set_app_setting(SETTING_ORDER.to_owned(), json))
}

/// Puts `items` in the order of `order`; items the list does not name keep
/// their relative order after the named ones. Unknown ids are ignored.
fn apply_order<T>(mut items: Vec<T>, order: &[String], id: impl Fn(&T) -> &str) -> Vec<T> {
    // A stable sort keeps the added order among the unnamed ones.
    items.sort_by_key(|item| {
        order
            .iter()
            .position(|wanted| wanted == id(item))
            .unwrap_or(usize::MAX)
    });
    items
}

/// A validated, de-duplicated copy of an order the frame sent.
fn clean_order(order: Vec<String>) -> Result<Vec<String>, MailError> {
    if order.len() > MAX_ACCOUNTS {
        return Err(MailError::InvalidInput);
    }
    let mut seen: Vec<String> = Vec::with_capacity(order.len());
    for id in order {
        if !mail::id_ok(&id) {
            return Err(MailError::InvalidInput);
        }
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    Ok(seen)
}

fn account_from_record(record: MailAccountRecord) -> Option<Account> {
    let auth = auth_of(&record.id);
    account_from_record_with(record, auth)
}

fn account_from_record_with(record: MailAccountRecord, auth: AuthKind) -> Option<Account> {
    let account = Account {
        id: record.id,
        display_name: record.display_name,
        email: record.email,
        imap: Endpoint {
            host: record.imap_host,
            port: record.imap_port,
            security: security_parse(&record.imap_security)?,
        },
        smtp: Endpoint {
            host: record.smtp_host,
            port: record.smtp_port,
            security: security_parse(&record.smtp_security)?,
        },
        username: record.username,
        colour: record.colour,
        signature: record.signature,
        save_sent_copy: record.save_sent_copy,
        auth,
    };
    account.validate().ok()?;
    Some(account)
}

fn role_out(role: FolderRole) -> MailFolderRoleV1 {
    match role {
        FolderRole::Inbox => MailFolderRoleV1::Inbox,
        FolderRole::Sent => MailFolderRoleV1::Sent,
        FolderRole::Drafts => MailFolderRoleV1::Drafts,
        FolderRole::Trash => MailFolderRoleV1::Trash,
        FolderRole::Archive => MailFolderRoleV1::Archive,
        FolderRole::Junk => MailFolderRoleV1::Junk,
        FolderRole::Other => MailFolderRoleV1::Other,
    }
}

fn folder_out(folder: &mail::Folder) -> MailFolderV1 {
    MailFolderV1 {
        path: folder.path.clone(),
        name: folder.name.clone(),
        delimiter: folder.delimiter.map(String::from),
        role: role_out(folder.role),
        unread: folder.unread,
        total: folder.total,
        selectable: folder.selectable,
    }
}

fn folder_record(folder: &mail::Folder) -> MailFolderRecord {
    MailFolderRecord {
        path: folder.path.clone(),
        name: folder.name.clone(),
        delimiter: folder.delimiter.map(String::from),
        role: folder.role.as_str().to_owned(),
        unread: folder.unread,
        total: folder.total,
        uid_validity: folder.uid_validity,
        uid_next: folder.uid_next,
        highest_modseq: folder.highest_modseq,
        selectable: folder.selectable,
    }
}

fn folder_from_record(record: &MailFolderRecord) -> MailFolderV1 {
    MailFolderV1 {
        path: record.path.clone(),
        name: record.name.clone(),
        delimiter: record.delimiter.clone(),
        role: role_out(FolderRole::parse(&record.role)),
        unread: record.unread,
        total: record.total,
        selectable: record.selectable,
    }
}

fn address_out(address: &Address) -> MailAddressV1 {
    MailAddressV1 {
        name: address.name.clone(),
        email: address.email.clone(),
    }
}

fn address_in(address: &MailAddressV1) -> Address {
    Address {
        name: address.name.clone(),
        email: address.email.trim().to_owned(),
    }
}

const FLAG_SEEN: u8 = 1;
const FLAG_FLAGGED: u8 = 2;
const FLAG_ANSWERED: u8 = 4;
const FLAG_DRAFT: u8 = 8;
const FLAG_DELETED: u8 = 16;

fn flag_bits(flags: mail::Flags) -> u8 {
    [
        (flags.seen, FLAG_SEEN),
        (flags.flagged, FLAG_FLAGGED),
        (flags.answered, FLAG_ANSWERED),
        (flags.draft, FLAG_DRAFT),
        (flags.deleted, FLAG_DELETED),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |bits, (_, bit)| bits | bit)
}

fn flags_from_bits(bits: u8) -> mail::Flags {
    mail::Flags {
        seen: bits & FLAG_SEEN != 0,
        flagged: bits & FLAG_FLAGGED != 0,
        answered: bits & FLAG_ANSWERED != 0,
        draft: bits & FLAG_DRAFT != 0,
        deleted: bits & FLAG_DELETED != 0,
    }
}

fn flag_kind(flag: MailFlagV1) -> (FlagKind, u8) {
    match flag {
        MailFlagV1::Seen => (FlagKind::Seen, FLAG_SEEN),
        MailFlagV1::Flagged => (FlagKind::Flagged, FLAG_FLAGGED),
        MailFlagV1::Deleted => (FlagKind::Deleted, FLAG_DELETED),
    }
}

fn message_out(header: &MessageHeader) -> MailMessageV1 {
    MailMessageV1 {
        uid: header.uid,
        message_id: header.message_id.clone(),
        subject: header.subject.clone(),
        from: header.from.iter().map(address_out).collect(),
        to: header.to.iter().map(address_out).collect(),
        cc: header.cc.iter().map(address_out).collect(),
        // Unix seconds are well inside f64's exact integer range.
        date: header.date.map(|seconds| seconds as f64),
        flags: MailFlagsV1 {
            seen: header.flags.seen,
            flagged: header.flags.flagged,
            answered: header.flags.answered,
            draft: header.flags.draft,
            deleted: header.flags.deleted,
        },
        size: header.size,
        has_attachments: header.has_attachments,
        snippet: header.snippet.clone(),
        in_reply_to: header.in_reply_to.clone(),
        references: header.references.clone(),
    }
}

fn message_record(header: &MessageHeader) -> Option<MailMessageRecord> {
    Some(MailMessageRecord {
        uid: header.uid,
        message_id: header.message_id.clone(),
        subject: header.subject.clone(),
        from_json: serde_json::to_string(&header.from).ok()?,
        to_json: serde_json::to_string(&header.to).ok()?,
        cc_json: serde_json::to_string(&header.cc).ok()?,
        date: header.date,
        flags: flag_bits(header.flags),
        size: header.size,
        has_attachments: header.has_attachments,
        snippet: header.snippet.clone(),
        in_reply_to: header.in_reply_to.clone(),
        references_json: serde_json::to_string(&header.references).ok()?,
    })
}

fn header_from_record(record: &MailMessageRecord) -> MessageHeader {
    MessageHeader {
        uid: record.uid,
        message_id: record.message_id.clone(),
        subject: record.subject.clone(),
        from: serde_json::from_str(&record.from_json).unwrap_or_default(),
        to: serde_json::from_str(&record.to_json).unwrap_or_default(),
        cc: serde_json::from_str(&record.cc_json).unwrap_or_default(),
        date: record.date,
        flags: flags_from_bits(record.flags),
        size: record.size,
        has_attachments: record.has_attachments,
        snippet: record.snippet.clone(),
        in_reply_to: record.in_reply_to.clone(),
        references: serde_json::from_str(&record.references_json).unwrap_or_default(),
    }
}

fn attachment_out(info: &AttachmentInfo) -> MailAttachmentV1 {
    MailAttachmentV1 {
        part: info.part,
        filename: info.filename.clone(),
        content_type: info.content_type.clone(),
        size: info.size,
        content_id: info.content_id.clone(),
        inline: info.inline,
    }
}

/// What the cache keeps beside the text: the attachment list and Reply-To.
#[derive(Serialize, Deserialize, Default)]
struct CachedExtras {
    attachments: Vec<AttachmentInfo>,
    reply_to: Vec<Address>,
}

fn body_out(body: &MessageBody) -> MailBodyV1 {
    use base64::Engine as _;
    MailBodyV1 {
        header: message_out(&body.header),
        reply_to: body.reply_to.iter().map(address_out).collect(),
        text: body.text.clone(),
        html: body.html.clone(),
        truncated: body.truncated,
        attachments: body.attachments.iter().map(attachment_out).collect(),
        inline_images: body
            .inline_images
            .iter()
            .map(|image| MailInlineImageV1 {
                content_id: image.content_id.clone(),
                content_type: image.content_type.clone(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(&image.data),
            })
            .collect(),
        invitations: body
            .invitation_parts()
            .into_iter()
            .map(|part| MailInvitationPartV1 {
                part: part.part,
                method: part.method.clone(),
                filename: part.filename.clone(),
                size: part.size,
            })
            .collect(),
    }
}

fn body_record(body: &MessageBody) -> Option<MailBodyRecord> {
    let extras = CachedExtras {
        attachments: body.attachments.clone(),
        reply_to: body.reply_to.clone(),
    };
    Some(MailBodyRecord {
        text: body.text.clone(),
        html: body.html.clone(),
        attachments_json: serde_json::to_string(&extras).ok()?,
        truncated: body.truncated,
    })
}

/// A cached body can stand in for a fetch unless the message has inline
/// images, which the cache does not keep.
fn body_from_cache(header: &MessageHeader, record: &MailBodyRecord) -> Option<MessageBody> {
    let extras: CachedExtras = serde_json::from_str(&record.attachments_json).ok()?;
    if extras.attachments.iter().any(|a| a.content_id.is_some()) {
        return None;
    }
    Some(MessageBody {
        header: header.clone(),
        reply_to: extras.reply_to,
        text: record.text.clone(),
        html: record.html.clone(),
        truncated: record.truncated,
        attachments: extras.attachments,
        inline_images: Vec::new(),
    })
}

fn draft_in(draft: &MailDraftV1) -> Reply<Draft> {
    use base64::Engine as _;
    let mut total = draft.text.len() + draft.html.as_ref().map_or(0, String::len);
    let mut attachments = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        total = total.saturating_add(attachment.data_base64.len());
        if total > MAX_DRAFT_BYTES {
            return Err(MailErrorV1::TooLarge);
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(&attachment.data_base64)
            .map_err(|_| MailErrorV1::InvalidInput)?;
        attachments.push(OutgoingAttachment {
            filename: attachment.filename.clone(),
            content_type: attachment.content_type.clone(),
            data,
        });
    }
    if total > MAX_DRAFT_BYTES {
        return Err(MailErrorV1::TooLarge);
    }
    Ok(Draft {
        to: draft.to.iter().map(address_in).collect(),
        cc: draft.cc.iter().map(address_in).collect(),
        bcc: draft.bcc.iter().map(address_in).collect(),
        subject: draft.subject.clone(),
        text: draft.text.clone(),
        html: draft.html.clone(),
        attachments,
        in_reply_to: draft.in_reply_to.clone(),
        references: draft.references.clone(),
        calendar: draft
            .calendar
            .as_ref()
            .map(|calendar| zephium_mail::model::CalendarBody {
                method: calendar.method.clone(),
                ics: calendar.ics.clone(),
            }),
    })
}

fn new_account_id() -> Reply<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| MailErrorV1::Internal)?;
    let mut id = String::with_capacity(33);
    id.push('m');
    for byte in bytes {
        id.push_str(&format!("{byte:02x}"));
    }
    Ok(id)
}

/// What a folder refresh changed. The inbox decides `NewMail`.
fn classify(
    before: &[MailFolderRecord],
    after: &[mail::Folder],
) -> Option<(MailChangeKindV1, String, u32, u32)> {
    let mut counts: Option<(String, u32)> = None;
    for folder in after {
        let Some(old) = before.iter().find(|old| old.path == folder.path) else {
            continue;
        };
        let arrived = match (old.uid_next, folder.uid_next) {
            (Some(old), Some(new)) if new > old => new - old,
            _ => 0,
        };
        if arrived > 0 && folder.role == FolderRole::Inbox {
            return Some((
                MailChangeKindV1::NewMail,
                folder.path.clone(),
                folder.unread,
                arrived,
            ));
        }
        if counts.is_none() && (old.unread != folder.unread || old.total != folder.total) {
            counts = Some((folder.path.clone(), folder.unread));
        }
    }
    counts.map(|(path, unread)| (MailChangeKindV1::Counts, path, unread, 0))
}

fn mailto_out(parsed: zephium_mail::mailto::Mailto) -> MailtoV1 {
    MailtoV1 {
        to: parsed.to,
        cc: parsed.cc,
        bcc: parsed.bcc,
        subject: parsed.subject,
        body: parsed.body,
    }
}

fn rule_in(rule: &MailRuleV1) -> Rule {
    Rule {
        field: match rule.field {
            MailRuleFieldV1::From => RuleField::From,
            MailRuleFieldV1::To => RuleField::To,
            MailRuleFieldV1::Subject => RuleField::Subject,
        },
        contains: rule.contains.trim().to_owned(),
        action: match rule.action {
            MailRuleActionV1::Move => RuleAction::Move,
            MailRuleActionV1::MarkRead => RuleAction::MarkRead,
            MailRuleActionV1::Flag => RuleAction::Flag,
        },
        folder: rule.folder.clone(),
    }
}

fn rule_out(rule: &Rule) -> MailRuleV1 {
    MailRuleV1 {
        field: match rule.field {
            RuleField::From => MailRuleFieldV1::From,
            RuleField::To => MailRuleFieldV1::To,
            RuleField::Subject => MailRuleFieldV1::Subject,
        },
        contains: rule.contains.clone(),
        action: match rule.action {
            RuleAction::Move => MailRuleActionV1::Move,
            RuleAction::MarkRead => MailRuleActionV1::MarkRead,
            RuleAction::Flag => MailRuleActionV1::Flag,
        },
        folder: rule.folder.clone(),
    }
}

/// Serializes the read-modify-write of the rule set.
static RULES_LOCK: Mutex<()> = Mutex::new(());

fn rule_set() -> rules::RuleSet {
    use zephium_core::ports::store::Store;
    APP_STORE
        .get()
        .and_then(|store| store.app_setting(SETTING_RULES))
        .map(|json| rules::parse_rule_set(&json))
        .unwrap_or_default()
}

fn rules_of(account: &str) -> Vec<Rule> {
    rule_set().remove(account).unwrap_or_default()
}

/// Saves one account's rules (an empty list removes them). Validation and the
/// size bound are enforced before anything is written.
fn store_rules(account: &str, new_rules: Vec<Rule>) -> Result<(), MailError> {
    use zephium_core::ports::store::Store;
    rules::validate_rules(&new_rules)?;
    let _guard = RULES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut set = rule_set();
    if new_rules.is_empty() {
        set.remove(account);
    } else {
        set.insert(account.to_owned(), new_rules);
    }
    let json = rules::serialize_rule_set(&set)?;
    if APP_STORE
        .get()
        .is_some_and(|store| store.set_app_setting(SETTING_RULES.to_owned(), json))
    {
        Ok(())
    } else {
        Err(MailError::Busy)
    }
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

struct Runtime {
    engine: MailEngine,
    watchers: Mutex<HashMap<String, std::sync::Arc<Notify>>>,
    /// Accounts whose watcher stopped because the server refused the sign-in.
    /// They stay stopped until the account is updated or the person refreshes.
    sign_in_failed: Mutex<std::collections::HashSet<String>>,
}

fn runtime() -> std::sync::Arc<Runtime> {
    static RUNTIME: OnceLock<std::sync::Arc<Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            std::sync::Arc::new(Runtime {
                engine: MailEngine::new(
                    std::sync::Arc::new(TlsConnectorImpl),
                    std::sync::Arc::new(OsVault),
                    std::sync::Arc::new(LettreSender),
                )
                .with_tokens(oauth::token_manager()),
                watchers: Mutex::new(HashMap::new()),
                sign_in_failed: Mutex::new(std::collections::HashSet::new()),
            })
        })
        .clone()
}

static ADMISSION: Semaphore = Semaphore::const_new(MAX_IN_FLIGHT);

/// Mail is on unless Settings switched it off. The Settings worker owns the
/// `mail.enabled` key; until it writes one this stays on, so the backend is
/// usable, and an explicit `false` shuts every command.
fn enabled() -> bool {
    use zephium_core::ports::store::Store;
    !matches!(
        APP_STORE
            .get()
            .and_then(|store| store.app_setting(SETTING_ENABLED))
            .as_deref(),
        Some("false" | "0" | "off")
    )
}

fn gate(
    caller: &WebviewWindow,
    command: &'static str,
) -> Reply<tokio::sync::SemaphorePermit<'static>> {
    if !authorize(caller, CallerPolicy::Main, command) || shutdown_started(caller.app_handle()) {
        return Err(MailErrorV1::Internal);
    }
    if !enabled() {
        return Err(MailErrorV1::Disabled);
    }
    ADMISSION.try_acquire().map_err(|_| MailErrorV1::Busy)
}

fn log(command: &str, ok: bool) {
    // Codes only: never an address, host, folder, subject or URL.
    write_diagnostic(format_args!("mail: command={command} ok={ok}"));
}

/// Runs `job` on the storage thread.
async fn db<R: Send + 'static>(
    job: impl FnOnce(&mut zephium_store::mail_cache::MailCache<'_>) -> CacheResult<R> + Send + 'static,
) -> Result<R, MailError> {
    let store = APP_STORE.get().ok_or(MailError::Internal)?.clone();
    tokio::task::spawn_blocking(move || store.with_mail_cache(job))
        .await
        .map_err(|_| MailError::Internal)?
        .ok_or(MailError::Busy)?
        .map_err(|_| MailError::Internal)
}

async fn load_account(id: &str) -> Result<Account, MailError> {
    if !mail::id_ok(id) {
        return Err(MailError::InvalidInput);
    }
    let wanted = id.to_owned();
    let record = db(move |cache| cache.account(&wanted)).await?;
    record
        .and_then(account_from_record)
        .ok_or(MailError::NotFound)
}

fn emit(app: &tauri::AppHandle, event: &MailChanged) {
    emit_to_privileged(app, MAIN_LABEL, EVENT, event);
}

async fn refresh_folders(
    app: &tauri::AppHandle,
    runtime: &Runtime,
    account: &Account,
) -> Result<Vec<mail::Folder>, MailError> {
    let mut live = runtime.engine.folders(account).await?;
    let id = account.id.clone();
    let records: Vec<MailFolderRecord> = live.iter().map(folder_record).collect();
    let before = db(move |cache| {
        let before = cache.folders(&id)?;
        cache.replace_folders(&id, &records)?;
        Ok(before)
    })
    .await?;
    let mut change = if before.is_empty() {
        None
    } else {
        classify(&before, &live)
    };
    if let Some((kind, folder, unread, arrived)) = change.as_mut() {
        if *kind == MailChangeKindV1::NewMail {
            let first_new = before
                .iter()
                .find(|old| old.path == *folder)
                .and_then(|old| old.uid_next);
            if let Some(first_new) = first_new {
                if let Some((fresh, silenced)) =
                    run_rules(app, runtime, account, folder, first_new, &live).await
                {
                    *unread = fresh
                        .iter()
                        .find(|candidate| candidate.path == *folder)
                        .map_or(*unread, |candidate| candidate.unread);
                    *arrived = arrived.saturating_sub(silenced);
                    if *arrived == 0 {
                        *kind = MailChangeKindV1::Counts;
                    }
                    live = fresh;
                }
            }
        }
    }
    if let Some((kind, folder, unread, new_messages)) = change {
        emit(
            app,
            &MailChanged {
                account: account.id.clone(),
                kind,
                folder: Some(folder),
                unread,
                new_messages,
            },
        );
    }
    Ok(live)
}

/// Runs the account's rules over the Inbox mail a sync just found (UIDs from
/// `first_new`). Returns the refreshed folder list and how many of the new
/// messages the rules took out of the alert (moved or marked read), or `None`
/// when no rule ran or none changed anything. Never fails a sync: the rules
/// are best effort and report nothing but a code to the diagnostic log.
async fn run_rules(
    app: &tauri::AppHandle,
    runtime: &Runtime,
    account: &Account,
    inbox: &str,
    first_new: u32,
    folders: &[mail::Folder],
) -> Option<(Vec<mail::Folder>, u32)> {
    let account_rules = rules_of(&account.id);
    if account_rules.is_empty() {
        return None;
    }
    let run = runtime
        .engine
        .apply_rules(account, inbox, &account_rules, first_new, folders)
        .await;
    log("rules", run.is_ok());
    let run = run.ok().filter(rules::RuleRun::changed)?;
    let (id, path) = (account.id.clone(), inbox.to_owned());
    let (seen, flagged, moved) = (run.seen.clone(), run.flagged.clone(), run.moved.clone());
    // The cache is an aid: it follows the server where it already has the rows.
    let _ = db(move |cache| {
        cache.update_flag_bit(&id, &path, &seen, FLAG_SEEN, true)?;
        cache.update_flag_bit(&id, &path, &flagged, FLAG_FLAGGED, true)?;
        cache.remove_messages(&id, &path, &moved)?;
        Ok(())
    })
    .await;
    if !run.moved.is_empty() {
        announce_removed(app, &account.id, inbox);
    }
    let mut silenced: Vec<u32> = run.seen.iter().chain(&run.moved).copied().collect();
    silenced.sort_unstable();
    silenced.dedup();
    let fresh = runtime.engine.folders(account).await.ok()?;
    let id = account.id.clone();
    let records: Vec<MailFolderRecord> = fresh.iter().map(folder_record).collect();
    let _ = db(move |cache| cache.replace_folders(&id, &records)).await;
    Some((fresh, u32::try_from(silenced.len()).unwrap_or(u32::MAX)))
}

fn spawn_watcher(
    app: tauri::AppHandle,
    runtime: std::sync::Arc<Runtime>,
    account_id: String,
    stop: std::sync::Arc<Notify>,
) {
    tauri::async_runtime::spawn(async move {
        const POLL: Duration = Duration::from_secs(120);
        let mut backoff = 5u64;
        loop {
            // Switched off or shutting down: end without touching the server.
            if shutdown_started(&app) || !enabled() {
                break;
            }
            let Ok(account) = load_account(&account_id).await else {
                break;
            };
            let pause = match runtime.engine.watch_inbox(&account, &stop).await {
                Ok(WatchOutcome::Stopped) => break,
                Ok(WatchOutcome::Changed) => {
                    backoff = 5;
                    let _ = refresh_folders(&app, &runtime, &account).await;
                    None
                }
                Ok(WatchOutcome::Idle) => {
                    backoff = 5;
                    None
                }
                // No IDLE: poll instead.
                Err(MailError::Unsupported) => {
                    let _ = refresh_folders(&app, &runtime, &account).await;
                    Some(POLL)
                }
                Err(error) => match watch_failure(error, backoff) {
                    WatchStep::StopSignIn => {
                        set_sign_in_failed(&app, &runtime, &account_id, true);
                        break;
                    }
                    WatchStep::Retry { wait, next } => {
                        backoff = next;
                        Some(wait)
                    }
                },
            };
            if let Some(wait) = pause {
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = stop.notified() => break,
                }
            }
        }
        if let Ok(mut watchers) = runtime.watchers.lock() {
            if watchers
                .get(&account_id)
                .is_some_and(|current| std::sync::Arc::ptr_eq(current, &stop))
            {
                watchers.remove(&account_id);
            }
        }
    });
}

/// How often the supervisor compares the running watchers with the accounts
/// and the `mail.enabled` switch.
const WATCH_RECONCILE: Duration = Duration::from_secs(15);

/// What the supervisor does: `(accounts to start, watchers to stop)`. With
/// Mail off every watcher stops and none starts; with Mail on each account
/// has one watcher and a watcher of a removed account stops.
fn watcher_plan(
    on: bool,
    running: &std::collections::HashSet<String>,
    accounts: &[String],
    blocked: &std::collections::HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    if !on {
        let mut stop: Vec<String> = running.iter().cloned().collect();
        stop.sort();
        return (Vec::new(), stop);
    }
    let start = accounts
        .iter()
        .filter(|id| !running.contains(*id) && !blocked.contains(*id))
        .cloned()
        .collect();
    let mut stop: Vec<String> = running
        .iter()
        .filter(|id| !accounts.contains(id))
        .cloned()
        .collect();
    stop.sort();
    (start, stop)
}

/// Longest wait between two watcher attempts after failures.
const WATCH_BACKOFF_CAP: u64 = 3600;

/// What a watcher does after a failed attempt.
#[derive(Debug, Eq, PartialEq)]
enum WatchStep {
    /// The server refused the sign-in: stop, and say so, until the account is
    /// updated or the person refreshes. Retrying would only hammer the server
    /// and may lock the account.
    StopSignIn,
    /// Any other failure: wait, doubling up to an hour.
    Retry { wait: Duration, next: u64 },
}

fn watch_failure(error: MailError, backoff: u64) -> WatchStep {
    match error {
        MailError::AuthFailed | MailError::Credentials => WatchStep::StopSignIn,
        _ => WatchStep::Retry {
            wait: Duration::from_secs(backoff),
            next: (backoff * 2).min(WATCH_BACKOFF_CAP),
        },
    }
}

fn is_sign_in_failed(account_id: &str) -> bool {
    runtime()
        .sign_in_failed
        .lock()
        .is_ok_and(|failed| failed.contains(account_id))
}

/// Records or clears the sign-in state and tells the frame when it changed.
fn set_sign_in_failed(app: &tauri::AppHandle, runtime: &Runtime, account_id: &str, failed: bool) {
    let changed = runtime.sign_in_failed.lock().is_ok_and(|mut set| {
        if failed {
            set.insert(account_id.to_owned())
        } else {
            set.remove(account_id)
        }
    });
    if changed {
        emit(
            app,
            &MailChanged {
                account: account_id.to_owned(),
                kind: MailChangeKindV1::Accounts,
                folder: None,
                unread: 0,
                new_messages: 0,
            },
        );
    }
}

/// Starts the account's watcher. A running one is kept unless `restart`.
fn start_watcher(
    app: &tauri::AppHandle,
    runtime: &std::sync::Arc<Runtime>,
    account_id: &str,
    restart: bool,
) -> Result<(), MailError> {
    if restart {
        // The account changed: a refused sign-in deserves a new try.
        set_sign_in_failed(app, runtime, account_id, false);
    }
    let mut watchers = runtime.watchers.lock().map_err(|_| MailError::Internal)?;
    if watchers.contains_key(account_id) && !restart {
        return Ok(());
    }
    if let Some(old) = watchers.remove(account_id) {
        old.notify_one();
    }
    let stop = std::sync::Arc::new(Notify::new());
    watchers.insert(account_id.to_owned(), stop.clone());
    drop(watchers);
    spawn_watcher(app.clone(), runtime.clone(), account_id.to_owned(), stop);
    Ok(())
}

fn stop_watchers(runtime: &Runtime, ids: &[String]) {
    if let Ok(mut watchers) = runtime.watchers.lock() {
        for id in ids {
            if let Some(stop) = watchers.remove(id) {
                stop.notify_one();
            }
        }
    }
}

async fn reconcile_watchers(app: &tauri::AppHandle, runtime: &std::sync::Arc<Runtime>) {
    let running: std::collections::HashSet<String> = runtime
        .watchers
        .lock()
        .map(|watchers| watchers.keys().cloned().collect())
        .unwrap_or_default();
    let on = enabled() && !shutdown_started(app);
    let accounts: Vec<String> = if on {
        match db(|cache| cache.accounts()).await {
            Ok(records) => records.into_iter().map(|record| record.id).collect(),
            // The store is busy: change nothing this round.
            Err(_) => return,
        }
    } else {
        Vec::new()
    };
    let blocked: std::collections::HashSet<String> = runtime
        .sign_in_failed
        .lock()
        .map(|set| set.clone())
        .unwrap_or_default();
    let (start, stop) = watcher_plan(on, &running, &accounts, &blocked);
    stop_watchers(runtime, &stop);
    for id in start {
        let _ = start_watcher(app, runtime, &id, false);
    }
}

/// Keeps one inbox watcher per account while Mail is on, independent of any
/// open frame: starts them at launch, when an account appears or Mail is
/// switched on, and stops them all when Mail is switched off and at shutdown.
pub(crate) fn install(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let runtime = runtime();
        loop {
            reconcile_watchers(&app, &runtime).await;
            if shutdown_started(&app) {
                break;
            }
            tokio::time::sleep(WATCH_RECONCILE).await;
        }
    });
}

// ---------------------------------------------------------------------------
// Autoconfig over HTTPS
// ---------------------------------------------------------------------------

struct HttpsFetcher(reqwest::Client);

impl HttpsFetcher {
    fn new() -> Option<Self> {
        // The same policy as the image fetcher: no proxy (it would resolve
        // names where the address check cannot see), public addresses only
        // at connect time, and every redirect passes `check_url` again, so a
        // redirect cannot lead to a private, loopback or link-local host.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent("Zephium")
            .no_proxy()
            .https_only(true)
            .referer(false)
            .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                // `previous` includes the first request.
                let followed = attempt.previous().len().saturating_sub(1);
                if remote_images::redirect_allowed(followed, attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .ok()?;
        Some(Self(client))
    }
}

impl ConfigFetcher for HttpsFetcher {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, MailError>> {
        Box::pin(async move {
            // Plain `https:` to a public host; the resolver checks the addresses.
            let url = remote_images::check_url(url)?;
            let mut response = self
                .0
                .get(url)
                .send()
                .await
                .map_err(|_| MailError::Offline)?;
            // A redirect the policy stopped arrives as a 3xx: not a config.
            if !response.status().is_success()
                || remote_images::check_url(response.url().as_str()).is_err()
            {
                return Err(MailError::NotFound);
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_AUTOCONFIG_BYTES as u64)
            {
                return Err(MailError::TooLarge);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| MailError::Offline)? {
                if body.len() + chunk.len() > MAX_AUTOCONFIG_BYTES {
                    return Err(MailError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(body)
        })
    }
}

// ---------------------------------------------------------------------------
// Remote images over HTTPS
// ---------------------------------------------------------------------------

/// Resolves a host name and keeps only public addresses, so a name that
/// points inside the person's network (or changes its answer between a check
/// and the connection) cannot be used to reach it.
struct PublicOnlyResolver;

fn public_only(addresses: Vec<std::net::SocketAddr>) -> Vec<std::net::SocketAddr> {
    addresses
        .into_iter()
        .filter(|address| remote_images::is_public_ip(address.ip()))
        .collect()
}

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found = tokio::task::spawn_blocking(move || {
                std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), 0))
                    .map(|addresses| addresses.collect::<Vec<_>>())
            })
            .await
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)??;
            let public = public_only(found);
            if public.is_empty() {
                return Err("no public address".into());
            }
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Fetches remote images: no cookies, no referrer, no proxy (a proxy would
/// resolve names where the address check cannot see), `https` only, a bounded
/// number of redirects that each pass the same URL policy.
struct ReqwestImages(reqwest::Client);

impl ReqwestImages {
    fn new() -> Option<Self> {
        let client = reqwest::Client::builder()
            .user_agent("Zephium")
            .no_proxy()
            .https_only(true)
            .referer(false)
            .connect_timeout(Duration::from_secs(6))
            .timeout(remote_images::REMOTE_IMAGE_TIMEOUT)
            .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                // `previous` includes the first request.
                let followed = attempt.previous().len().saturating_sub(1);
                if remote_images::redirect_allowed(followed, attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .ok()?;
        Some(Self(client))
    }
}

impl ImageFetcher for ReqwestImages {
    fn fetch<'a>(
        &'a self,
        url: &'a reqwest::Url,
        max_bytes: usize,
    ) -> BoxFuture<'a, Result<FetchedImage, MailError>> {
        Box::pin(async move {
            // The policy again, in case a caller skipped it.
            remote_images::check_url(url.as_str())?;
            let mut response = self
                .0
                .get(url.clone())
                .header("accept", "image/png,image/jpeg,image/gif,image/webp")
                .send()
                .await
                .map_err(|error| {
                    if error.is_timeout() {
                        MailError::Timeout
                    } else {
                        MailError::Offline
                    }
                })?;
            // A redirect that the policy stopped arrives as a 3xx: not an image.
            if !response.status().is_success()
                || remote_images::check_url(response.url().as_str()).is_err()
            {
                return Err(MailError::NotFound);
            }
            if response
                .content_length()
                .is_some_and(|length| length > max_bytes as u64)
            {
                return Err(MailError::TooLarge);
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| MailError::Offline)? {
                if bytes.len() + chunk.len() > max_bytes {
                    return Err(MailError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(FetchedImage {
                content_type,
                bytes,
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Looks up server settings for an address.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_autoconfig(
    caller: WebviewWindow,
    email: String,
) -> Reply<MailAutoconfigV1> {
    let _permit = gate(&caller, "mail_autoconfig")?;
    let fetcher = HttpsFetcher::new().ok_or(MailErrorV1::Internal)?;
    let found = autoconfig::discover(email.trim(), &fetcher).await;
    log("autoconfig", found.is_ok());
    Ok(match found? {
        Autoconfig::Found {
            imap,
            smtp,
            username,
            provider,
            source,
        } => MailAutoconfigV1 {
            found: true,
            imap: Some(endpoint_out(&imap)),
            smtp: Some(endpoint_out(&smtp)),
            username: Some(username),
            provider,
            source: Some(match source {
                AutoconfigSource::Ispdb => MailAutoconfigSourceV1::Ispdb,
                AutoconfigSource::ProviderSubdomain => MailAutoconfigSourceV1::ProviderSubdomain,
                AutoconfigSource::WellKnown => MailAutoconfigSourceV1::WellKnown,
            }),
        },
        Autoconfig::Manual => MailAutoconfigV1 {
            found: false,
            imap: None,
            smtp: None,
            username: None,
            provider: None,
            source: None,
        },
    })
}

/// The auth kind a sign-in handle stands for, without consuming it.
fn auth_from_grant(handle: &str) -> Result<AuthKind, MailError> {
    let (config, _) = oauth::peek_grant(handle).ok_or(MailError::InvalidInput)?;
    Ok(AuthKind::Oauth {
        provider: config.provider,
        client_id: config.client_id,
        tenant: config.tenant,
    })
}

fn account_from_new(input: &MailNewAccountV1, id: String, auth: AuthKind) -> Account {
    let email = input.email.trim().to_owned();
    Account {
        id,
        display_name: input.display_name.trim().to_owned(),
        // XOAUTH2 names the mailbox by its address.
        username: if auth == AuthKind::Password {
            input.username.trim().to_owned()
        } else {
            email.clone()
        },
        email,
        imap: endpoint_in(&input.imap),
        smtp: endpoint_in(&input.smtp),
        colour: input.colour.clone(),
        signature: input.signature.clone(),
        save_sent_copy: input.save_sent_copy,
        auth,
    }
}

/// Connects to both servers with the typed password, storing nothing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_test_account(
    caller: WebviewWindow,
    account: MailNewAccountV1,
) -> Reply<MailTestV1> {
    let _permit = gate(&caller, "mail_test_account")?;
    let result = async {
        match &account.oauth_grant {
            Some(handle) => {
                let auth = auth_from_grant(handle)?;
                let candidate = account_from_new(&account, "test".to_owned(), auth);
                let (_, token) = oauth::peek_grant(handle).ok_or(MailError::InvalidInput)?;
                runtime()
                    .engine
                    .test_credential(&candidate, &Credential::Bearer(token))
                    .await
            }
            None => {
                let candidate = account_from_new(&account, "test".to_owned(), AuthKind::Password);
                runtime()
                    .engine
                    .test_account(&candidate, &account.password)
                    .await
            }
        }
    }
    .await;
    log("test_account", result.is_ok());
    let report = result?;
    Ok(MailTestV1 {
        imap: report.imap.map(Into::into),
        smtp: report.smtp.map(Into::into),
    })
}

/// Adds an account. The password goes to the OS vault and is wiped here.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_add_account(
    caller: WebviewWindow,
    account: MailNewAccountV1,
) -> Reply<MailAccountV1> {
    let _permit = gate(&caller, "mail_add_account")?;
    let app = caller.app_handle().clone();
    let outcome = add_account(&app, &account).await;
    log("add_account", outcome.is_ok());
    outcome
        .map(|account| account_out(&account))
        .map_err(Into::into)
}

async fn add_account(
    app: &tauri::AppHandle,
    input: &MailNewAccountV1,
) -> Result<Account, MailError> {
    let auth = match &input.oauth_grant {
        Some(handle) => auth_from_grant(handle)?,
        None => AuthKind::Password,
    };
    let account = account_from_new(
        input,
        new_account_id().map_err(|_| MailError::Internal)?,
        auth,
    );
    account.validate()?;
    if db(|cache| cache.accounts()).await?.len() >= MAX_ACCOUNTS {
        return Err(MailError::Busy);
    }
    let vault = OsVault;
    match &input.oauth_grant {
        Some(handle) => {
            // The refresh token goes straight to the OS vault.
            let grant = oauth::take_grant(handle).ok_or(MailError::InvalidInput)?;
            oauth::token_manager().adopt(&account.id, &grant).await?;
            if !set_auth(&account.id, Some(account.auth.clone())) {
                let _ = oauth::token_manager().sign_out(&account.id).await;
                return Err(MailError::Busy);
            }
        }
        None => vault.set(&account.id, &input.password)?,
    }
    let record = account_record(&account);
    let saved = db(move |cache| cache.save_account(&record)).await;
    if !matches!(saved, Ok(true)) {
        let _ = vault.remove(&account.id);
        if input.oauth_grant.is_some() {
            let _ = oauth::token_manager().sign_out(&account.id).await;
            set_auth(&account.id, None);
        }
        return Err(saved.err().unwrap_or(MailError::Busy));
    }
    emit(
        app,
        &MailChanged {
            account: account.id.clone(),
            kind: MailChangeKindV1::Accounts,
            folder: None,
            unread: 0,
            new_messages: 0,
        },
    );
    let _ = start_watcher(app, &runtime(), &account.id, false);
    Ok(account)
}

/// Changes an account; a password, when given, replaces the stored one.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_update_account(
    caller: WebviewWindow,
    update: MailAccountUpdateV1,
) -> Reply<MailAccountV1> {
    let _permit = gate(&caller, "mail_update_account")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let current = load_account(&update.id).await?;
        // A grant re-signs an OAuth account in; a password belongs to a
        // password account. Never both, never the wrong kind.
        let oauth_account = matches!(current.auth, AuthKind::Oauth { .. });
        if (update.oauth_grant.is_some() && !oauth_account)
            || (update.password.is_some() && (oauth_account || update.oauth_grant.is_some()))
        {
            return Err(MailError::InvalidInput);
        }
        let auth = match &update.oauth_grant {
            Some(handle) => auth_from_grant(handle)?,
            None => current.auth.clone(),
        };
        let account = Account {
            id: update.id.clone(),
            display_name: update.display_name.trim().to_owned(),
            email: update.email.trim().to_owned(),
            imap: endpoint_in(&update.imap),
            smtp: endpoint_in(&update.smtp),
            username: update.username.trim().to_owned(),
            colour: update.colour.clone(),
            signature: update.signature.clone(),
            save_sent_copy: update.save_sent_copy,
            auth,
        };
        let account = Account {
            // XOAUTH2 names the mailbox by its address.
            username: if oauth_account {
                account.email.clone()
            } else {
                account.username.clone()
            },
            ..account
        };
        account.validate()?;
        if let Some(handle) = &update.oauth_grant {
            let grant = oauth::take_grant(handle).ok_or(MailError::InvalidInput)?;
            oauth::token_manager().adopt(&account.id, &grant).await?;
            if !set_auth(&account.id, Some(account.auth.clone())) {
                return Err(MailError::Busy);
            }
        }
        if let Some(password) = &update.password {
            OsVault.set(&account.id, password)?;
        }
        let record = account_record(&account);
        if !db(move |cache| cache.save_account(&record)).await? {
            return Err(MailError::Busy);
        }
        // Server settings or the password may have changed: start clean.
        runtime().engine.forget(&account.id).await;
        // The watcher holds its own connection: restart it on the new settings.
        let _ = start_watcher(&app, &runtime(), &account.id, true);
        emit(
            &app,
            &MailChanged {
                account: account.id.clone(),
                kind: MailChangeKindV1::Accounts,
                folder: None,
                unread: 0,
                new_messages: 0,
            },
        );
        Ok::<_, MailError>(account)
    }
    .await;
    log("update_account", outcome.is_ok());
    outcome
        .map(|account| account_out(&account))
        .map_err(Into::into)
}

/// Removes an account, its password and everything cached for it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_remove_account(caller: WebviewWindow, account: String) -> Reply<()> {
    let _permit = gate(&caller, "mail_remove_account")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        if !mail::id_ok(&account) {
            return Err(MailError::InvalidInput);
        }
        let runtime = runtime();
        if let Some(stop) = runtime
            .watchers
            .lock()
            .map_err(|_| MailError::Internal)?
            .remove(&account)
        {
            stop.notify_one();
        }
        if let Ok(mut failed) = runtime.sign_in_failed.lock() {
            failed.remove(&account);
        }
        runtime.engine.forget(&account).await;
        OsVault.remove(&account)?;
        oauth::token_manager().sign_out(&account).await?;
        set_auth(&account, None);
        {
            // Forget its place in the order (best effort).
            let _guard = ORDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let order = account_order();
            if order.contains(&account) {
                let kept: Vec<String> = order.into_iter().filter(|id| *id != account).collect();
                store_account_order(&kept);
            }
        }
        let id = account.clone();
        if !db(move |cache| cache.remove_account(&id)).await? {
            return Err(MailError::NotFound);
        }
        emit(
            &app,
            &MailChanged {
                account,
                kind: MailChangeKindV1::Accounts,
                folder: None,
                unread: 0,
                new_messages: 0,
            },
        );
        Ok(())
    }
    .await;
    log("remove_account", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// The accounts in the person's order (`mail.account_order`), then any
/// others in the order they were added. No secrets.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_accounts(caller: WebviewWindow) -> Reply<Vec<MailAccountV1>> {
    let _permit = gate(&caller, "mail_accounts")?;
    let records = db(|cache| cache.accounts()).await?;
    let accounts: Vec<MailAccountV1> = records
        .into_iter()
        .filter_map(account_from_record)
        .map(|account| account_out(&account))
        .collect();
    Ok(apply_order(accounts, &account_order(), |account| {
        &account.id
    }))
}

/// Saves the order the person arranged the accounts in. Unknown ids are
/// harmless (ignored when reading); malformed ones are refused.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_set_account_order(caller: WebviewWindow, order: Vec<String>) -> Reply<()> {
    let _permit = gate(&caller, "mail_set_account_order")?;
    let outcome = async {
        let order = clean_order(order)?;
        let _guard = ORDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if store_account_order(&order) {
            Ok(())
        } else {
            Err(MailError::Busy)
        }
    }
    .await;
    log("set_account_order", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Folders from the local cache; reads the server only when nothing is cached.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_folders(
    caller: WebviewWindow,
    account: String,
) -> Reply<Vec<MailFolderV1>> {
    let _permit = gate(&caller, "mail_folders")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let account = load_account(&account).await?;
        let id = account.id.clone();
        let cached = db(move |cache| cache.folders(&id)).await?;
        if !cached.is_empty() {
            return Ok(cached.iter().map(folder_from_record).collect());
        }
        let live = refresh_folders(&app, &runtime(), &account).await?;
        Ok::<_, MailError>(live.iter().map(folder_out).collect())
    }
    .await;
    log("folders", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Refreshes folders and counts from the server and announces changes.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_sync(caller: WebviewWindow, account: String) -> Reply<Vec<MailFolderV1>> {
    let _permit = gate(&caller, "mail_sync")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let account = load_account(&account).await?;
        let runtime = runtime();
        let live = refresh_folders(&app, &runtime, &account).await?;
        // The server accepted the sign-in: the person's refresh also revives a
        // watcher that had stopped on a refusal.
        set_sign_in_failed(&app, &runtime, &account.id, false);
        let _ = start_watcher(&app, &runtime, &account.id, false);
        Ok::<_, MailError>(live.iter().map(folder_out).collect())
    }
    .await;
    log("sync", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// A page of headers, newest first. Falls back to the cache while offline.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_messages(
    caller: WebviewWindow,
    account: String,
    folder: String,
    page: MailPageRequestV1,
) -> Reply<MailPageV1> {
    let _permit = gate(&caller, "mail_messages")?;
    let outcome = async {
        let account = load_account(&account).await?;
        let limit = page.limit.clamp(1, MAX_PAGE);
        match runtime()
            .engine
            .headers(&account, &folder, page.before_uid, limit)
            .await
        {
            Ok(live) => {
                let rows: Vec<MailMessageRecord> =
                    live.messages.iter().filter_map(message_record).collect();
                let (id, path, validity) = (account.id.clone(), folder.clone(), live.uid_validity);
                // The cache is an aid; failing to fill it never fails the read.
                let _ = db(move |cache| cache.store_headers(&id, &path, validity, &rows)).await;
                Ok(MailPageV1 {
                    messages: live.messages.iter().map(message_out).collect(),
                    next_before: live.next_before,
                    total: live.total,
                    from_cache: false,
                })
            }
            Err(error @ (MailError::Offline | MailError::Timeout | MailError::Tls)) => {
                let (id, path, before) = (account.id.clone(), folder.clone(), page.before_uid);
                let cached = db(move |cache| cache.cached_headers(&id, &path, before, limit))
                    .await
                    .unwrap_or_default();
                if cached.is_empty() {
                    return Err(error);
                }
                let next_before = (cached.len() as u32 >= limit)
                    .then(|| cached.last().map(|row| row.uid))
                    .flatten();
                Ok(MailPageV1 {
                    total: cached.len() as u32,
                    messages: cached
                        .iter()
                        .map(|row| message_out(&header_from_record(row)))
                        .collect(),
                    next_before,
                    from_cache: true,
                })
            }
            Err(error) => Err(error),
        }
    }
    .await;
    log("messages", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// One message with its body. Bodies are cached after the first read.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_message(
    caller: WebviewWindow,
    account: String,
    folder: String,
    uid: u32,
) -> Reply<MailBodyV1> {
    let _permit = gate(&caller, "mail_message")?;
    let outcome = async {
        let account = load_account(&account).await?;
        Ok::<_, MailError>(body_out(&read_body(&account, &folder, uid).await?))
    }
    .await;
    log("message", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// A message body from the cache, or from the server (and then cached).
async fn read_body(account: &Account, folder: &str, uid: u32) -> Result<MessageBody, MailError> {
    let (id, path) = (account.id.clone(), folder.to_owned());
    let cached = db(move |cache| {
        let body = cache.cached_body(&id, &path, uid)?;
        let header = cache
            .cached_headers(&id, &path, uid.checked_add(1), 1)?
            .into_iter()
            .find(|row| row.uid == uid);
        Ok(body.zip(header))
    })
    .await
    .unwrap_or(None);
    if let Some((record, header)) = cached {
        if let Some(body) = body_from_cache(&header_from_record(&header), &record) {
            return Ok(body);
        }
    }
    let body = runtime().engine.message(account, folder, uid).await?;
    if let Some(record) = body_record(&body) {
        let (id, path) = (account.id.clone(), folder.to_owned());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0));
        let _ = db(move |cache| cache.store_body(&id, &path, uid, &record, now)).await;
    }
    Ok(body)
}

/// Fetches the remote images ("Load images") of one message as `data:` URLs.
/// Only `https:` images that the message's own HTML references are fetched,
/// with bounded count, size, time and redirects, from public addresses only,
/// without cookies or credentials; anything that cannot be fetched safely is
/// left out. The frame's CSP stays closed: it only ever shows `data:` images.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_fetch_remote_images(
    caller: WebviewWindow,
    account: String,
    folder: String,
    uid: u32,
    urls: Vec<String>,
) -> Reply<Vec<MailRemoteImageV1>> {
    let _permit = gate(&caller, "mail_fetch_remote_images")?;
    let outcome = async {
        let account = load_account(&account).await?;
        let body = read_body(&account, &folder, uid).await?;
        let html = body.html.as_deref().ok_or(MailError::InvalidInput)?;
        let fetcher = ReqwestImages::new().ok_or(MailError::Internal)?;
        remote_images::fetch_remote_images(html, &urls, &fetcher).await
    }
    .await;
    log("fetch_remote_images", outcome.is_ok());
    outcome
        .map(|images| {
            images
                .into_iter()
                .map(|image| MailRemoteImageV1 {
                    url: image.url,
                    data_url: image.data_url,
                })
                .collect()
        })
        .map_err(Into::into)
}

/// The bytes of one calendar part, base64, for `calendar_invitation_parse`.
/// Only a part the message listed as a calendar invitation is handed out,
/// at most 1 MiB; nothing is written to disk and nothing is sent.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_invitation_bytes(
    caller: WebviewWindow,
    account: String,
    folder: String,
    uid: u32,
    part: u32,
) -> Reply<String> {
    use base64::Engine as _;
    let _permit = gate(&caller, "mail_invitation_bytes")?;
    let outcome = async {
        let account = load_account(&account).await?;
        let (info, bytes) = runtime()
            .engine
            .attachment(&account, &folder, uid, part)
            .await?;
        if !info.is_calendar() || bytes.len() > zephium_mail::limits::MAX_INVITATION_BYTES {
            return Err(MailError::InvalidInput);
        }
        Ok::<_, MailError>(base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    .await;
    log("invitation_bytes", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Saves one attachment where the person chooses. The bytes never reach the frame.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_attachment(
    caller: WebviewWindow,
    account: String,
    folder: String,
    uid: u32,
    part: u32,
) -> Reply<MailSaveOutcomeV1> {
    use tauri_plugin_dialog::DialogExt;
    let _permit = gate(&caller, "mail_attachment")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let account = load_account(&account).await?;
        let (info, bytes) = runtime()
            .engine
            .attachment(&account, &folder, uid, part)
            .await?;
        let name = info.filename.clone();
        let picked = tokio::task::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_file_name(&name)
                .blocking_save_file()
        })
        .await
        .map_err(|_| MailError::Internal)?;
        let Some(path) = picked.and_then(|picked| picked.into_path().ok()) else {
            return Ok(MailSaveOutcomeV1::Cancelled);
        };
        tokio::task::spawn_blocking(move || write_attachment(&path, &bytes))
            .await
            .map_err(|_| MailError::Internal)?
            .map_err(|_| MailError::Internal)?;
        Ok::<_, MailError>(MailSaveOutcomeV1::Saved)
    }
    .await;
    log("attachment", outcome.is_ok());
    outcome.map_err(Into::into)
}

fn write_attachment(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing directory"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn announce_removed(app: &tauri::AppHandle, account: &str, folder: &str) {
    emit(
        app,
        &MailChanged {
            account: account.to_owned(),
            kind: MailChangeKindV1::Removed,
            folder: Some(folder.to_owned()),
            unread: 0,
            new_messages: 0,
        },
    );
}

/// Sets or clears `\Seen`, `\Flagged` or `\Deleted` on messages.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_set_flags(
    caller: WebviewWindow,
    account: String,
    folder: String,
    uids: Vec<u32>,
    flag: MailFlagV1,
    on: bool,
) -> Reply<()> {
    let _permit = gate(&caller, "mail_set_flags")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let account = load_account(&account).await?;
        let (kind, bit) = flag_kind(flag);
        runtime()
            .engine
            .set_flag(&account, &folder, &uids, kind, on)
            .await?;
        let (id, path, uids) = (account.id.clone(), folder.clone(), uids);
        let _ = db(move |cache| cache.update_flag_bit(&id, &path, &uids, bit, on)).await;
        emit(
            &app,
            &MailChanged {
                account: account.id,
                kind: MailChangeKindV1::Counts,
                folder: Some(folder),
                unread: 0,
                new_messages: 0,
            },
        );
        Ok::<_, MailError>(())
    }
    .await;
    log("set_flags", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Moves messages to another folder.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_move(
    caller: WebviewWindow,
    account: String,
    folder: String,
    uids: Vec<u32>,
    destination: String,
) -> Reply<()> {
    let _permit = gate(&caller, "mail_move")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let account = load_account(&account).await?;
        runtime()
            .engine
            .move_messages(&account, &folder, &uids, &destination)
            .await?;
        let (id, path, gone) = (account.id.clone(), folder.clone(), uids);
        let _ = db(move |cache| cache.remove_messages(&id, &path, &gone)).await;
        announce_removed(&app, &account.id, &folder);
        Ok::<_, MailError>(())
    }
    .await;
    log("move", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Deletes messages: into Trash, or for good from Trash. An account with no
/// Trash folder answers `no_trash` and removes nothing, unless `permanent` is
/// true, which the frame sets only after the person confirmed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_delete(
    caller: WebviewWindow,
    account: String,
    folder: String,
    uids: Vec<u32>,
    permanent: bool,
) -> Reply<()> {
    let _permit = gate(&caller, "mail_delete")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let account = load_account(&account).await?;
        runtime()
            .engine
            .delete(&account, &folder, &uids, permanent)
            .await?;
        let (id, path, gone) = (account.id.clone(), folder.clone(), uids);
        let _ = db(move |cache| cache.remove_messages(&id, &path, &gone)).await;
        announce_removed(&app, &account.id, &folder);
        Ok::<_, MailError>(())
    }
    .await;
    log("delete", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Sends a message, then files a copy in Sent when the account asks for it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_send(
    caller: WebviewWindow,
    account: String,
    draft: MailDraftV1,
) -> Reply<MailSendOutcomeV1> {
    let _permit = gate(&caller, "mail_send")?;
    let outcome = async {
        let account = load_account(&account).await?;
        let draft = draft_in(&draft).map_err(|_| MailError::InvalidInput)?;
        let sent = runtime().engine.send(&account, &draft).await?;
        Ok::<_, MailError>(MailSendOutcomeV1 {
            sent: true,
            saved_copy: sent.saved_copy,
        })
    }
    .await;
    log("send", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Stores a draft in the Drafts folder.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_save_draft(
    caller: WebviewWindow,
    account: String,
    draft: MailDraftV1,
) -> Reply<()> {
    let _permit = gate(&caller, "mail_save_draft")?;
    let outcome = async {
        let account = load_account(&account).await?;
        let draft = draft_in(&draft).map_err(|_| MailError::InvalidInput)?;
        runtime().engine.save_draft(&account, &draft).await
    }
    .await;
    log("save_draft", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Searches one folder on the server by subject, sender and text.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_search(
    caller: WebviewWindow,
    account: String,
    folder: String,
    query: String,
) -> Reply<Vec<MailMessageV1>> {
    let _permit = gate(&caller, "mail_search")?;
    let outcome = async {
        let account = load_account(&account).await?;
        let found = runtime().engine.search(&account, &folder, &query).await?;
        Ok::<_, MailError>(found.iter().map(message_out).collect())
    }
    .await;
    log("search", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Addresses from cached mail that match what is being typed, best first.
/// Reads only the local cache: no network, bounded rows and results.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_contacts(
    caller: WebviewWindow,
    prefix: String,
    limit: u32,
) -> Reply<Vec<MailContactV1>> {
    let _permit = gate(&caller, "mail_contacts")?;
    let outcome = async {
        let limit = limit.clamp(1, zephium_store::mail_cache::MAX_CONTACT_RESULTS);
        let found = db(move |cache| cache.contacts(&prefix, limit)).await?;
        Ok::<_, MailError>(
            found
                .into_iter()
                .map(|contact| MailContactV1 {
                    name: contact.name,
                    email: contact.email,
                })
                .collect(),
        )
    }
    .await;
    log("contacts", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// One account's rules, in the order they run.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_rules(caller: WebviewWindow, account: String) -> Reply<Vec<MailRuleV1>> {
    let _permit = gate(&caller, "mail_rules")?;
    let outcome = async {
        let account = load_account(&account).await?;
        Ok::<_, MailError>(rules_of(&account.id).iter().map(rule_out).collect())
    }
    .await;
    log("rules_read", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Replaces one account's rules. Invalid or oversized lists are refused whole.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_set_rules(
    caller: WebviewWindow,
    account: String,
    rules: Vec<MailRuleV1>,
) -> Reply<()> {
    let _permit = gate(&caller, "mail_set_rules")?;
    let outcome = async {
        let account = load_account(&account).await?;
        if rules.len() > zephium_mail::rules::MAX_RULES_PER_ACCOUNT {
            return Err(MailError::TooLarge);
        }
        let rules: Vec<Rule> = rules.iter().map(rule_in).collect();
        store_rules(&account.id, rules)
    }
    .await;
    log("set_rules", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// Parses a `mailto:` link for a compose window (RFC 6068, bounded; a link
/// with a line break in a header field or an address that is not plain is
/// refused). Used for links inside a message.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_parse_mailto(caller: WebviewWindow, url: String) -> Reply<MailtoV1> {
    let _permit = gate(&caller, "mail_parse_mailto")?;
    let outcome = zephium_mail::mailto::parse(&url).map(mailto_out);
    log("parse_mailto", outcome.is_ok());
    outcome.map_err(Into::into)
}

/// True for a `mailto:` link the person clicked in a page: the one kind of
/// external-app request Mail may take over.
pub(crate) fn is_mailto_request(event: &zephium_core::ports::engine::EngineEvent) -> bool {
    use zephium_core::ports::engine::EngineEvent;
    matches!(
        event,
        EngineEvent::ExternalAppRequested { url, user_gesture: true, .. }
            if zephium_mail::mailto::is_mailto(url)
    )
}

/// Routes a clicked `mailto:` link to Zephium's compose when Mail is on and an
/// account exists and the link parses. Otherwise `fallback` gets the event
/// back unchanged, so the usual "open in another application" card appears.
pub(crate) fn route_mailto(
    app: tauri::AppHandle,
    event: zephium_core::ports::engine::EngineEvent,
    fallback: impl FnOnce(zephium_core::ports::engine::EngineEvent) + Send + 'static,
) {
    use zephium_core::ports::engine::EngineEvent;
    tauri::async_runtime::spawn(async move {
        let handled = match &event {
            EngineEvent::ExternalAppRequested { url, .. } => open_mailto(&app, url).await,
            _ => false,
        };
        log("mailto", handled);
        if !handled {
            fallback(event);
        }
    });
}

async fn open_mailto(app: &tauri::AppHandle, url: &str) -> bool {
    if shutdown_started(app) || !enabled() {
        return false;
    }
    let Ok(parsed) = zephium_mail::mailto::parse(url) else {
        return false;
    };
    match db(|cache| cache.accounts()).await {
        Ok(accounts) if !accounts.is_empty() => {}
        _ => return false,
    }
    let request = MailComposeRequested {
        mailto: mailto_out(parsed),
    };
    if !try_emit_to_privileged(app, MAIN_LABEL, EVENT_COMPOSE, &request) {
        return false;
    }
    let _ = execute_command(app, "mode.mail");
    true
}

/// Starts or stops watching an account's inbox for new mail. The watcher
/// announces changes through `zephium:mail-changed`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn mail_watch(caller: WebviewWindow, account: String, on: bool) -> Reply<()> {
    let _permit = gate(&caller, "mail_watch")?;
    let app = caller.app_handle().clone();
    let outcome = async {
        let account = load_account(&account).await?;
        let runtime = runtime();
        if on {
            // The backend already watches every account; this is idempotent,
            // except that asking again revives a watcher stopped on a refusal.
            set_sign_in_failed(&app, &runtime, &account.id, false);
            start_watcher(&app, &runtime, &account.id, false)?;
        } else {
            stop_watchers(&runtime, &[account.id]);
        }
        Ok::<_, MailError>(())
    }
    .await;
    log("watch", outcome.is_ok());
    outcome.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str, uid_next: u32, unread: u32, total: u32) -> mail::Folder {
        mail::Folder {
            path: path.into(),
            name: path.into(),
            delimiter: Some('/'),
            role: if path == "INBOX" {
                FolderRole::Inbox
            } else {
                FolderRole::Other
            },
            unread,
            total,
            uid_validity: Some(1),
            uid_next: Some(uid_next),
            highest_modseq: None,
            selectable: true,
        }
    }

    fn record(path: &str, uid_next: u32, unread: u32, total: u32) -> MailFolderRecord {
        MailFolderRecord {
            path: path.into(),
            name: path.into(),
            delimiter: None,
            role: "other".into(),
            unread,
            total,
            uid_validity: Some(1),
            uid_next: Some(uid_next),
            highest_modseq: None,
            selectable: true,
        }
    }

    #[test]
    fn a_higher_uidnext_on_the_inbox_is_new_mail_and_other_moves_are_counts() {
        let before = [record("INBOX", 10, 1, 9), record("Archive", 4, 0, 3)];
        let new_mail = [folder("INBOX", 13, 4, 12), folder("Archive", 4, 0, 3)];
        assert_eq!(
            classify(&before, &new_mail),
            Some((MailChangeKindV1::NewMail, "INBOX".into(), 4, 3))
        );
        let read_one = [folder("INBOX", 10, 0, 9), folder("Archive", 4, 0, 3)];
        assert_eq!(
            classify(&before, &read_one),
            Some((MailChangeKindV1::Counts, "INBOX".into(), 0, 0))
        );
        let same = [folder("INBOX", 10, 1, 9), folder("Archive", 4, 0, 3)];
        assert_eq!(classify(&before, &same), None);
        let archive_grew = [folder("INBOX", 10, 1, 9), folder("Archive", 6, 0, 5)];
        assert_eq!(
            classify(&before, &archive_grew),
            Some((MailChangeKindV1::Counts, "Archive".into(), 0, 0)),
            "only the inbox raises NewMail"
        );
        assert_eq!(classify(&[], &new_mail), None);
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn every_account_gets_a_watcher_at_start_and_none_is_doubled() {
        let none = std::collections::HashSet::new();
        let (start, stop) = watcher_plan(true, &none, &ids(&["a", "b"]), &none);
        assert_eq!((start, stop), (ids(&["a", "b"]), Vec::new()));
        let running: std::collections::HashSet<String> = ids(&["a"]).into_iter().collect();
        let (start, stop) = watcher_plan(true, &running, &ids(&["a", "b"]), &none);
        assert_eq!((start, stop), (ids(&["b"]), Vec::new()));
    }

    #[test]
    fn switching_mail_off_stops_every_watcher_and_on_restarts_them() {
        let running: std::collections::HashSet<String> = ids(&["b", "a"]).into_iter().collect();
        let none = std::collections::HashSet::new();
        let (start, stop) = watcher_plan(false, &running, &ids(&["a", "b", "c"]), &none);
        assert_eq!((start, stop), (Vec::new(), ids(&["a", "b"])));
        let (start, stop) = watcher_plan(
            true,
            &std::collections::HashSet::new(),
            &ids(&["a", "b"]),
            &none,
        );
        assert_eq!((start, stop), (ids(&["a", "b"]), Vec::new()));
    }

    #[test]
    fn a_removed_accounts_watcher_stops() {
        let running: std::collections::HashSet<String> = ids(&["a", "gone"]).into_iter().collect();
        let none = std::collections::HashSet::new();
        let (start, stop) = watcher_plan(true, &running, &ids(&["a"]), &none);
        assert_eq!((start, stop), (Vec::new(), ids(&["gone"])));
    }

    #[test]
    fn an_account_whose_sign_in_was_refused_is_not_restarted_by_the_supervisor() {
        let none = std::collections::HashSet::new();
        let blocked: std::collections::HashSet<String> = ids(&["b"]).into_iter().collect();
        let (start, stop) = watcher_plan(true, &none, &ids(&["a", "b"]), &blocked);
        assert_eq!((start, stop), (ids(&["a"]), Vec::new()));
    }

    #[test]
    fn failed_watchers_back_off_up_to_an_hour_and_a_refused_sign_in_stops_them() {
        let mut backoff = 5;
        let mut waits = Vec::new();
        for _ in 0..12 {
            let WatchStep::Retry { wait, next } = watch_failure(MailError::Offline, backoff) else {
                panic!("a network failure retries");
            };
            waits.push(wait.as_secs());
            backoff = next;
        }
        assert_eq!(waits[..4], [5, 10, 20, 40]);
        assert_eq!(*waits.last().expect("waits"), WATCH_BACKOFF_CAP);
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]));
        for error in [MailError::AuthFailed, MailError::Credentials] {
            assert_eq!(watch_failure(error, 5), WatchStep::StopSignIn);
        }
        for error in [MailError::Timeout, MailError::Tls, MailError::Protocol] {
            assert!(matches!(watch_failure(error, 5), WatchStep::Retry { .. }));
        }
    }

    #[test]
    fn flags_round_trip_through_the_cache_bits() {
        let flags = mail::Flags {
            seen: true,
            flagged: false,
            answered: true,
            draft: false,
            deleted: true,
        };
        assert_eq!(flag_bits(flags), FLAG_SEEN | FLAG_ANSWERED | FLAG_DELETED);
        assert_eq!(flags_from_bits(flag_bits(flags)), flags);
    }

    fn header() -> MessageHeader {
        MessageHeader {
            uid: 7,
            message_id: Some("m@x".into()),
            subject: "Hi".into(),
            from: vec![Address {
                name: Some("Ann".into()),
                email: "ann@example.com".into(),
            }],
            to: vec![Address {
                name: None,
                email: "bob@example.com".into(),
            }],
            date: Some(1_731_499_200),
            flags: mail::Flags {
                seen: true,
                ..mail::Flags::default()
            },
            size: 10,
            snippet: "hello".into(),
            references: vec!["a@x".into()],
            ..MessageHeader::default()
        }
    }

    #[test]
    fn a_header_survives_the_cache_round_trip() {
        let original = header();
        let record = message_record(&original).expect("record");
        assert_eq!(header_from_record(&record), original);
    }

    #[test]
    fn a_cached_body_stands_in_unless_the_message_has_inline_images() {
        let attachment = |content_id: Option<&str>| AttachmentInfo {
            part: 0,
            filename: "a.pdf".into(),
            content_type: "application/pdf".into(),
            size: 3,
            content_id: content_id.map(str::to_owned),
            inline: content_id.is_some(),
            method: None,
        };
        let mut body = MessageBody {
            header: header(),
            reply_to: vec![Address {
                name: None,
                email: "list@example.com".into(),
            }],
            text: Some("plain".into()),
            html: Some("<p>x</p>".into()),
            attachments: vec![attachment(None)],
            ..MessageBody::default()
        };
        let record = body_record(&body).expect("record");
        let restored = body_from_cache(&header(), &record).expect("cache serves it");
        assert_eq!(
            restored.reply_to[0].email, "list@example.com",
            "Reply-To is kept"
        );
        assert_eq!(restored.attachments, body.attachments);
        body.attachments.push(attachment(Some("logo@x")));
        let with_inline = body_record(&body).expect("record");
        assert!(body_from_cache(&header(), &with_inline).is_none());
        let garbage = MailBodyRecord {
            attachments_json: "nope".into(),
            ..MailBodyRecord::default()
        };
        assert!(body_from_cache(&header(), &garbage).is_none());
    }

    #[test]
    fn account_records_validate_on_the_way_out_and_refuse_plain_text() {
        let account = Account {
            id: "m00".into(),
            display_name: "Ann".into(),
            email: "ann@example.com".into(),
            imap: Endpoint {
                host: "imap.example.com".into(),
                port: 993,
                security: Security::Tls,
            },
            smtp: Endpoint {
                host: "smtp.example.com".into(),
                port: 587,
                security: Security::StartTls,
            },
            username: "ann".into(),
            colour: "#336699".into(),
            signature: String::new(),
            save_sent_copy: true,
            auth: AuthKind::Password,
        };
        let record = account_record(&account);
        assert_eq!(account_from_record(record.clone()), Some(account.clone()));
        let oauth_kind = AuthKind::Oauth {
            provider: zephium_oauth::provider::Provider::Google,
            client_id: "123-abc.apps.googleusercontent.com".into(),
            tenant: None,
        };
        let signed_in =
            account_from_record_with(record.clone(), oauth_kind.clone()).expect("oauth");
        assert_eq!(signed_in.auth, oauth_kind);
        let out = serde_json::to_string(&account_out(&signed_in)).expect("json");
        assert!(out.contains("\"oauth\""));
        assert!(!out.contains("token"));
        let plain_out = serde_json::to_string(&account_out(&account)).expect("json");
        assert!(plain_out.contains("\"oauth\":null"));
        let mut plain = record.clone();
        plain.imap_security = "plain".into();
        assert!(account_from_record(plain).is_none());
        let mut hostile = record;
        hostile.imap_host = "evil host".into();
        assert!(account_from_record(hostile).is_none());
    }

    #[test]
    fn the_dto_carries_no_password_and_errors_carry_no_text() {
        let account = Account {
            id: "m00".into(),
            display_name: "Ann".into(),
            email: "ann@example.com".into(),
            imap: Endpoint {
                host: "h.example.com".into(),
                port: 993,
                security: Security::Tls,
            },
            smtp: Endpoint {
                host: "h.example.com".into(),
                port: 465,
                security: Security::Tls,
            },
            username: "ann".into(),
            colour: "#000000".into(),
            signature: String::new(),
            save_sent_copy: false,
            auth: AuthKind::Password,
        };
        let json = serde_json::to_string(&account_out(&account)).expect("json");
        assert!(!json.contains("password"));
        assert_eq!(
            serde_json::to_string(&MailErrorV1::from(MailError::AuthFailed)).expect("json"),
            "\"auth_failed\""
        );
    }

    #[test]
    fn drafts_are_decoded_and_bounded() {
        use base64::Engine as _;
        let draft = |data: String| MailDraftV1 {
            to: vec![MailAddressV1 {
                name: None,
                email: " bob@example.com ".into(),
            }],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "s".into(),
            text: "t".into(),
            html: None,
            attachments: vec![MailOutgoingAttachmentV1 {
                filename: "a.txt".into(),
                content_type: "text/plain".into(),
                data_base64: data,
            }],
            in_reply_to: None,
            references: Vec::new(),
            calendar: None,
        };
        let ok = draft_in(&draft(
            base64::engine::general_purpose::STANDARD.encode(b"hi"),
        ))
        .expect("draft");
        assert_eq!(ok.attachments[0].data, b"hi");
        assert_eq!(ok.to[0].email, "bob@example.com");
        assert_eq!(
            draft_in(&draft("***".into())),
            Err(MailErrorV1::InvalidInput)
        );
        assert_eq!(
            draft_in(&draft("A".repeat(MAX_DRAFT_BYTES + 1))),
            Err(MailErrorV1::TooLarge)
        );
    }

    #[test]
    fn account_ids_are_valid_and_unique() {
        let first = new_account_id().expect("id");
        let second = new_account_id().expect("id");
        assert!(mail::id_ok(&first));
        assert_ne!(first, second);
    }

    #[test]
    fn accounts_follow_the_saved_order_and_the_rest_keep_their_place() {
        let ids = |items: Vec<&str>| items.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let order = ids(vec!["c", "gone", "a"]);
        let sorted = apply_order(ids(vec!["a", "b", "c", "d"]), &order, |id| id.as_str());
        assert_eq!(sorted, ids(vec!["c", "a", "b", "d"]));
        let untouched = apply_order(ids(vec!["a", "b"]), &[], |id| id.as_str());
        assert_eq!(untouched, ids(vec!["a", "b"]));
    }

    #[test]
    fn a_saved_order_is_validated_bounded_and_deduplicated() {
        let ids = |items: &[&str]| items.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            clean_order(ids(&["m1", "m2", "m1"])).expect("order"),
            ids(&["m1", "m2"])
        );
        assert_eq!(
            clean_order(Vec::new()).expect("empty"),
            Vec::<String>::new()
        );
        assert_eq!(clean_order(ids(&["bad id"])), Err(MailError::InvalidInput));
        assert_eq!(clean_order(ids(&[""])), Err(MailError::InvalidInput));
        assert_eq!(clean_order(ids(&["../x"])), Err(MailError::InvalidInput));
        let many = (0..=MAX_ACCOUNTS).map(|n| format!("m{n}")).collect();
        assert_eq!(clean_order(many), Err(MailError::InvalidInput));
    }

    #[test]
    fn the_autoconfig_client_refuses_private_targets_and_redirects_to_them() {
        let fetcher = HttpsFetcher::new().expect("client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        for url in [
            "http://autoconfig.example.com/config.xml",
            "https://127.0.0.1/config.xml",
            "https://[::1]/config.xml",
            "https://169.254.169.254/latest/meta-data",
            "https://192.168.1.1/config.xml",
            "https://localhost/config.xml",
            "https://router.local/config.xml",
        ] {
            let result = runtime.block_on(fetcher.fetch(url));
            assert_eq!(result, Err(MailError::InvalidInput), "{url}");
        }
        // Redirect hops obey the same URL policy as the first request.
        let hop = |next: &str| {
            remote_images::redirect_allowed(0, &reqwest::Url::parse(next).expect("url"))
        };
        assert!(hop("https://autoconfig.provider.example/v1.1/example.com"));
        for next in [
            "https://10.0.0.5/x",
            "https://127.0.0.1/x",
            "https://169.254.169.254/x",
            "https://[fe80::1]/x",
            "https://intranet/x",
            "http://autoconfig.provider.example/x",
        ] {
            assert!(!hop(next), "{next}");
        }
    }

    #[test]
    fn the_image_resolver_drops_every_non_public_address() {
        let addresses = [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.9",
            "169.254.1.1",
            "::1",
            "fe80::1",
            "93.184.216.34",
        ]
        .iter()
        .map(|ip| std::net::SocketAddr::new(ip.parse().expect("ip"), 0))
        .collect();
        let kept = public_only(addresses);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].ip().to_string(), "93.184.216.34");
        assert!(public_only(vec!["10.0.0.1:0".parse().expect("addr")]).is_empty());
    }

    #[test]
    fn the_image_client_builds_and_refuses_unsafe_urls_before_any_request() {
        let fetcher = ReqwestImages::new().expect("client");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        for url in [
            "http://example.com/a.png",
            "https://127.0.0.1/a.png",
            "https://localhost/a.png",
        ] {
            let parsed = reqwest::Url::parse(url).expect("url");
            let result = runtime.block_on(fetcher.fetch(&parsed, 1024));
            assert_eq!(result, Err(MailError::InvalidInput), "{url}");
        }
    }

    #[test]
    fn writing_an_attachment_replaces_atomically() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"old").expect("seed");
        write_attachment(&path, b"new").expect("write");
        assert_eq!(std::fs::read(&path).expect("read"), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 1);
    }

    #[test]
    fn rules_cross_the_boundary_both_ways_and_are_validated_before_storing() {
        let wire = MailRuleV1 {
            field: MailRuleFieldV1::Subject,
            contains: "  invoice ".into(),
            action: MailRuleActionV1::Move,
            folder: Some("Bills".into()),
        };
        let rule = rule_in(&wire);
        assert_eq!(rule.contains, "invoice", "the text is trimmed");
        assert_eq!(
            rule_out(&rule),
            MailRuleV1 {
                contains: "invoice".into(),
                ..wire
            }
        );
        // Refusals happen before the store is touched (there is none in a unit test).
        let bad = [
            Rule {
                contains: String::new(),
                ..rule.clone()
            },
            Rule {
                folder: None,
                ..rule.clone()
            },
            Rule {
                contains: "a
Bcc: x"
                    .into(),
                ..rule.clone()
            },
        ];
        for bad in bad {
            assert_eq!(
                store_rules("acc-1", vec![bad]),
                Err(MailError::InvalidInput)
            );
        }
        let many = vec![rule; rules::MAX_RULES_PER_ACCOUNT + 1];
        assert_eq!(store_rules("acc-1", many), Err(MailError::TooLarge));
    }

    #[test]
    fn only_a_clicked_mailto_link_is_taken_over() {
        use zephium_core::ports::engine::EngineEvent;
        let event = |url: &str, user_gesture: bool| EngineEvent::ExternalAppRequested {
            id: zephium_core::ids::ItemId::generate(),
            url: url.into(),
            app: None,
            user_gesture,
            initiator: zephium_core::navigation::AppLinkSource::Page,
        };
        assert!(is_mailto_request(&event("mailto:a@example.com", true)));
        assert!(is_mailto_request(&event(
            "MAILTO:a@example.com?subject=x",
            true
        )));
        assert!(!is_mailto_request(&event("mailto:a@example.com", false)));
        assert!(!is_mailto_request(&event("zoommtg://zoom.us/join", true)));
        assert!(!is_mailto_request(&EngineEvent::RuntimeRestartRequired));
    }

    #[test]
    fn a_parsed_link_carries_only_the_compose_fields() {
        let wire = mailto_out(
            zephium_mail::mailto::parse("mailto:a@example.com?cc=b@example.com&subject=Hi&body=Yo")
                .expect("parse"),
        );
        assert_eq!(wire.to, ["a@example.com"]);
        assert_eq!(wire.cc, ["b@example.com"]);
        assert!(wire.bcc.is_empty());
        assert_eq!((wire.subject.as_str(), wire.body.as_str()), ("Hi", "Yo"));
    }

    #[test]
    fn a_body_lists_its_calendar_parts_and_a_draft_carries_only_a_reply_alternative() {
        let mut body = MessageBody {
            header: header(),
            ..MessageBody::default()
        };
        body.attachments = vec![
            AttachmentInfo {
                part: 0,
                filename: "a.pdf".into(),
                content_type: "application/pdf".into(),
                size: 3,
                content_id: None,
                inline: false,
                method: None,
            },
            AttachmentInfo {
                part: 1,
                filename: "invite.ics".into(),
                content_type: "text/calendar".into(),
                size: 90,
                content_id: None,
                inline: false,
                method: Some("REQUEST".into()),
            },
        ];
        let wire = body_out(&body);
        assert_eq!(
            wire.invitations,
            vec![MailInvitationPartV1 {
                part: 1,
                method: Some("REQUEST".into()),
                filename: "invite.ics".into(),
                size: 90,
            }]
        );
        let draft = MailDraftV1 {
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Accepted: Plan".into(),
            text: "ok".into(),
            html: None,
            attachments: vec![],
            in_reply_to: None,
            references: vec![],
            calendar: Some(MailCalendarBodyV1 {
                method: "REPLY".into(),
                ics: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".into(),
            }),
        };
        let converted = draft_in(&draft).expect("converts");
        assert_eq!(
            converted.calendar.as_ref().map(|c| c.method.as_str()),
            Some("REPLY")
        );
    }
}
