//! The mail engine: one reusable IMAP connection per account with bounded
//! timeouts and a single reconnect, SMTP submission with an optional copy
//! into Sent, and a separate IDLE connection for `watch_inbox`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{Mutex as AsyncMutex, Notify};

use zephium_oauth::manager::AccessTokenSource;
use zephium_oauth::provider::Purpose;

use crate::auth::{AuthKind, Credential};
use crate::error::{MailError, Result};
use crate::imap::ImapSession;
use crate::limits::MAX_ACCOUNTS;
use crate::mime;
use crate::model::{
    Account, AttachmentInfo, Draft, FlagKind, Folder, FolderRole, HeaderPage, MessageBody,
    MessageHeader, WatchOutcome,
};
use crate::smtp::{compose, SmtpSender};
use crate::transport::Connector;
use crate::vault::PasswordVault;

impl MailError {
    /// True when the connection itself is suspect and a fresh one may help.
    #[must_use]
    pub fn is_connection(self) -> bool {
        matches!(self, Self::Offline | Self::Timeout | Self::Protocol)
    }
}

type Slot = Arc<AsyncMutex<Option<ImapSession>>>;

/// Result of sending: the message went out; `saved_copy` says whether the
/// copy in Sent was also stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SendOutcome {
    /// True when the Sent copy was appended.
    pub saved_copy: bool,
}

/// What testing an account's two servers found. `None` means it worked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountTest {
    /// Incoming server.
    pub imap: Option<MailError>,
    /// Outgoing server.
    pub smtp: Option<MailError>,
}

/// The engine. Cheap to share behind an `Arc`.
pub struct MailEngine {
    connector: Arc<dyn Connector>,
    vault: Arc<dyn PasswordVault>,
    smtp: Arc<dyn SmtpSender>,
    tokens: Option<Arc<dyn AccessTokenSource>>,
    slots: Mutex<HashMap<String, Slot>>,
}

impl std::fmt::Debug for MailEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("MailEngine").finish_non_exhaustive()
    }
}

macro_rules! with_session {
    ($engine:expr, $account:expr, |$session:ident| $operation:expr) => {{
        let slot = $engine.slot(&$account.id)?;
        let mut guard = slot.lock().await;
        let mut retried = false;
        loop {
            if guard.is_none() {
                *guard = Some($engine.connect($account).await?);
            }
            let $session = guard.as_mut().ok_or(MailError::Internal)?;
            let outcome = $operation.await;
            // A server that sent more than a command allows leaves the stream
            // out of step: never reuse that connection, whatever the outcome.
            if guard.as_ref().is_some_and(ImapSession::is_poisoned) {
                *guard = None;
            }
            match outcome {
                Ok(value) => break Ok(value),
                Err(error) if error.is_connection() => {
                    *guard = None;
                    if retried {
                        break Err(error);
                    }
                    retried = true;
                }
                Err(error) => break Err(error),
            }
        }
    }};
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

impl MailEngine {
    /// Builds an engine over its three ports.
    #[must_use]
    pub fn new(
        connector: Arc<dyn Connector>,
        vault: Arc<dyn PasswordVault>,
        smtp: Arc<dyn SmtpSender>,
    ) -> Self {
        Self {
            connector,
            vault,
            smtp,
            tokens: None,
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Adds the source of OAuth access tokens. Without one, OAuth accounts
    /// fail with [`MailError::Credentials`] and password accounts work as before.
    #[must_use]
    pub fn with_tokens(mut self, tokens: Arc<dyn AccessTokenSource>) -> Self {
        self.tokens = Some(tokens);
        self
    }

    /// What to present to the account's servers right now: its password, or
    /// a bearer token that is refreshed first when it is about to expire.
    async fn credential(&self, account: &Account) -> Result<Credential> {
        match &account.auth {
            AuthKind::Password => Ok(Credential::Password(self.vault.get(&account.id)?)),
            kind @ AuthKind::Oauth { .. } => {
                let config = kind.oauth_config().ok_or(MailError::Internal)?;
                let tokens = self.tokens.as_ref().ok_or(MailError::Credentials)?;
                let token = tokens
                    .access_token(&account.id, &config, Purpose::Mail)
                    .await?;
                Ok(Credential::Bearer(token))
            }
        }
    }

    fn slot(&self, account_id: &str) -> Result<Slot> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if !slots.contains_key(account_id) && slots.len() >= MAX_ACCOUNTS {
            return Err(MailError::Busy);
        }
        Ok(slots.entry(account_id.to_owned()).or_default().clone())
    }

    async fn connect(&self, account: &Account) -> Result<ImapSession> {
        account.validate()?;
        let credential = self.credential(account).await?;
        ImapSession::connect(
            &*self.connector,
            &account.imap,
            &account.username,
            &credential,
        )
        .await
    }

    /// Drops the account's cached connection (after removal or a password change).
    pub async fn forget(&self, account_id: &str) {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account_id);
        if let Some(slot) = slot {
            if let Some(session) = slot.lock().await.take() {
                session.logout().await;
            }
        }
    }

    /// Tests both servers with a password that is not stored yet.
    pub async fn test_account(&self, account: &Account, password: &str) -> Result<AccountTest> {
        self.test_credential(account, &Credential::password(password))
            .await
    }

    /// Tests both servers with a credential that is not stored yet (a
    /// password, or the access token of a sign-in not yet attached to an
    /// account).
    pub async fn test_credential(
        &self,
        account: &Account,
        credential: &Credential,
    ) -> Result<AccountTest> {
        account.validate()?;
        let imap = match ImapSession::connect(
            &*self.connector,
            &account.imap,
            &account.username,
            credential,
        )
        .await
        {
            Ok(session) => {
                session.logout().await;
                None
            }
            Err(error) => Some(error),
        };
        let smtp = self
            .smtp
            .check(&account.smtp, &account.username, credential)
            .await
            .err();
        Ok(AccountTest { imap, smtp })
    }

    /// Folders with roles and unread counts. This is also the refresh used by sync.
    pub async fn folders(&self, account: &Account) -> Result<Vec<Folder>> {
        with_session!(self, account, |session| session.list_folders(true))
    }

    /// One page of message headers, newest first.
    pub async fn headers(
        &self,
        account: &Account,
        folder: &str,
        before_uid: Option<u32>,
        limit: u32,
    ) -> Result<HeaderPage> {
        with_session!(self, account, |session| session
            .headers(folder, before_uid, limit))
    }

    /// One message with its body.
    pub async fn message(&self, account: &Account, folder: &str, uid: u32) -> Result<MessageBody> {
        with_session!(self, account, |session| session.fetch_message(folder, uid))
    }

    /// One attachment's bytes.
    pub async fn attachment(
        &self,
        account: &Account,
        folder: &str,
        uid: u32,
        part: u32,
    ) -> Result<(AttachmentInfo, Vec<u8>)> {
        let raw = with_session!(self, account, |session| session.fetch_raw(folder, uid))?;
        mime::attachment_bytes(&raw, part)
    }

    /// Sets or clears a flag.
    pub async fn set_flag(
        &self,
        account: &Account,
        folder: &str,
        uids: &[u32],
        flag: FlagKind,
        on: bool,
    ) -> Result<()> {
        with_session!(self, account, |session| session
            .set_flag(folder, uids, flag, on))
    }

    /// Moves messages to another folder.
    pub async fn move_messages(
        &self,
        account: &Account,
        folder: &str,
        uids: &[u32],
        dest: &str,
    ) -> Result<()> {
        with_session!(self, account, |session| session
            .move_messages(folder, uids, dest))
    }

    /// Deletes messages: into Trash, or for good when already in Trash.
    ///
    /// When the account has no recognised Trash folder nothing is removed and
    /// [`MailError::NoTrash`] comes back, unless `permanent` says the person
    /// confirmed a permanent delete.
    pub async fn delete(
        &self,
        account: &Account,
        folder: &str,
        uids: &[u32],
        permanent: bool,
    ) -> Result<()> {
        let trash = self.role_path(account, FolderRole::Trash).await?;
        match trash {
            Some(trash) if trash != folder => {
                with_session!(self, account, |session| session
                    .move_messages(folder, uids, &trash))
            }
            Some(_) => with_session!(self, account, |session| session.expunge_uids(folder, uids)),
            None if permanent => {
                with_session!(self, account, |session| session.expunge_uids(folder, uids))
            }
            None => Err(MailError::NoTrash),
        }
    }

    /// Searches subject, sender and text of one folder.
    pub async fn search(
        &self,
        account: &Account,
        folder: &str,
        query: &str,
    ) -> Result<Vec<MessageHeader>> {
        with_session!(self, account, |session| session.search(folder, query))
    }

    async fn role_path(&self, account: &Account, role: FolderRole) -> Result<Option<String>> {
        let folders = with_session!(self, account, |session| session.list_folders(false))?;
        Ok(folders
            .into_iter()
            .find(|folder| folder.role == role)
            .map(|folder| folder.path))
    }

    /// Sends a message, then appends a copy to Sent when the account asks
    /// for it. A failed copy never fails a message that went out.
    pub async fn send(&self, account: &Account, draft: &Draft) -> Result<SendOutcome> {
        let composed = compose(account, draft, false, now())?;
        if composed.recipients.is_empty() {
            return Err(MailError::InvalidInput);
        }
        let credential = self.credential(account).await?;
        self.smtp
            .send(&account.smtp, &account.username, &credential, &composed)
            .await?;
        let mut saved_copy = false;
        if account.save_sent_copy {
            if let Ok(Some(sent)) = self.role_path(account, FolderRole::Sent).await {
                let raw = &composed.raw;
                saved_copy = with_session!(self, account, |session| session.append(
                    &sent,
                    &["\\Seen"],
                    raw
                ))
                .is_ok();
            }
        }
        Ok(SendOutcome { saved_copy })
    }

    /// Stores the draft in the Drafts folder.
    pub async fn save_draft(&self, account: &Account, draft: &Draft) -> Result<()> {
        let composed = compose(account, draft, true, now())?;
        let drafts = self
            .role_path(account, FolderRole::Drafts)
            .await?
            .ok_or(MailError::NotFound)?;
        let raw = &composed.raw;
        with_session!(self, account, |session| session.append(
            &drafts,
            &["\\Draft", "\\Seen"],
            raw
        ))
    }

    /// Waits on IDLE for the inbox on its own connection until it changes,
    /// the renewal interval passes, or `stop` is notified.
    pub async fn watch_inbox(&self, account: &Account, stop: &Notify) -> Result<WatchOutcome> {
        let session = self.connect(account).await?;
        let (session, outcome) = session.watch("INBOX", stop).await?;
        session.logout().await;
        Ok(outcome)
    }
}
