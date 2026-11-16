//! Protocol flows through the real engine against the in-process fake server.

mod fake_imap;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fake_imap::{simple, standard_folders, FakeConnector, Hostile, PASSWORD, TOKEN};
use futures::future::BoxFuture;
use tokio::sync::Notify;
use zephium_mail::auth::{AuthKind, Credential};
use zephium_mail::engine::MailEngine;
use zephium_mail::error::{MailError, Result};
use zephium_mail::model::{
    Account, Address, Draft, Endpoint, FlagKind, FolderRole, Security, WatchOutcome,
};
use zephium_mail::smtp::{Composed, SmtpSender};
use zephium_mail::vault::{MemoryVault, PasswordVault};

#[derive(Default)]
struct FakeSmtp {
    sent: Mutex<Vec<Composed>>,
    credentials: Mutex<Vec<String>>,
    fail: Mutex<Option<MailError>>,
}

impl SmtpSender for FakeSmtp {
    fn send<'a>(
        &'a self,
        _e: &'a Endpoint,
        _u: &'a str,
        credential: &'a Credential,
        composed: &'a Composed,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.credentials
                .lock()
                .unwrap()
                .push(format!("{credential:?}:{}", credential.secret()));
            if let Some(error) = *self.fail.lock().unwrap() {
                return Err(error);
            }
            self.sent.lock().unwrap().push(composed.clone());
            Ok(())
        })
    }
    fn check<'a>(
        &'a self,
        _e: &'a Endpoint,
        _u: &'a str,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if credential.secret() == PASSWORD || credential.secret() == TOKEN {
                Ok(())
            } else {
                Err(MailError::AuthFailed)
            }
        })
    }
}

fn account(save_sent: bool) -> Account {
    let endpoint = |port| Endpoint {
        host: "mail.example.com".into(),
        port,
        security: Security::Tls,
    };
    Account {
        id: "acc-1".into(),
        display_name: "Me".into(),
        email: "me@example.com".into(),
        imap: endpoint(993),
        smtp: endpoint(465),
        username: "me".into(),
        colour: "#112233".into(),
        signature: String::new(),
        save_sent_copy: save_sent,
        auth: AuthKind::Password,
    }
}

fn oauth_account() -> Account {
    Account {
        auth: AuthKind::Oauth {
            provider: zephium_oauth::provider::Provider::Microsoft,
            client_id: "client-1".into(),
            tenant: None,
        },
        ..account(false)
    }
}

/// Hands out a fixed token and counts how often it was asked.
struct FakeTokens {
    token: &'static str,
    asked: std::sync::atomic::AtomicUsize,
    fail: Mutex<Option<zephium_oauth::OAuthError>>,
}

impl FakeTokens {
    fn new(token: &'static str) -> Arc<Self> {
        Arc::new(Self {
            token,
            asked: std::sync::atomic::AtomicUsize::new(0),
            fail: Mutex::new(None),
        })
    }
}

impl zephium_oauth::manager::AccessTokenSource for FakeTokens {
    fn access_token<'a>(
        &'a self,
        account_id: &'a str,
        config: &'a zephium_oauth::provider::ClientConfig,
        purpose: zephium_oauth::provider::Purpose,
    ) -> BoxFuture<'a, zephium_oauth::Result<zeroize::Zeroizing<String>>> {
        Box::pin(async move {
            assert_eq!(account_id, "acc-1");
            assert_eq!(config.client_id, "client-1");
            assert_eq!(purpose, zephium_oauth::provider::Purpose::Mail);
            self.asked.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = *self.fail.lock().unwrap() {
                return Err(error);
            }
            Ok(zeroize::Zeroizing::new(self.token.to_owned()))
        })
    }
}

struct Rig {
    engine: MailEngine,
    server: Arc<FakeConnector>,
    smtp: Arc<FakeSmtp>,
    vault: Arc<MemoryVault>,
}

fn rig() -> Rig {
    let server = FakeConnector::new(standard_folders());
    let smtp = Arc::new(FakeSmtp::default());
    let vault = Arc::new(MemoryVault::default());
    vault.set("acc-1", PASSWORD).unwrap();
    let engine = MailEngine::new(server.clone(), vault.clone(), smtp.clone());
    Rig {
        engine,
        server,
        smtp,
        vault,
    }
}

fn fill_inbox(rig: &Rig, count: u32) {
    let mut state = rig.server.state.lock().unwrap();
    for n in 1..=count {
        state.folders[0].add(
            if n % 2 == 0 { &["\\Seen"] } else { &[] },
            &simple(
                "Ann <ann@example.com>",
                &format!("S{n}"),
                &format!("body {n} needle"),
            ),
        );
    }
}

fn addr(email: &str) -> Address {
    Address {
        name: None,
        email: email.into(),
    }
}

#[tokio::test]
async fn lists_folders_with_special_use_roles_and_counts() {
    let rig = rig();
    fill_inbox(&rig, 3);
    let folders = rig.engine.folders(&account(true)).await.unwrap();
    let role = |path: &str| folders.iter().find(|f| f.path == path).unwrap().role;
    assert_eq!(role("INBOX"), FolderRole::Inbox);
    assert_eq!(role("Sent Items"), FolderRole::Sent);
    assert_eq!(
        role("Entwürfe"),
        FolderRole::Drafts,
        "decoded modified UTF-7, role from its name"
    );
    assert_eq!(role("Trash"), FolderRole::Trash);
    assert_eq!(role("Archive"), FolderRole::Archive);
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap();
    assert_eq!(
        (
            inbox.total,
            inbox.unread,
            inbox.uid_validity,
            inbox.uid_next
        ),
        (3, 2, Some(7), Some(4))
    );
    assert_eq!(inbox.highest_modseq, Some(42));
    assert!(
        !folders
            .iter()
            .find(|f| f.path == "Projects")
            .unwrap()
            .selectable
    );
}

#[tokio::test]
async fn pages_headers_newest_first_by_uid_range() {
    let rig = rig();
    fill_inbox(&rig, 5);
    let a = account(true);
    let first = rig.engine.headers(&a, "INBOX", None, 2).await.unwrap();
    assert_eq!(
        first.messages.iter().map(|m| m.uid).collect::<Vec<_>>(),
        [5, 4]
    );
    assert_eq!(first.next_before, Some(4));
    assert_eq!(first.total, 5);
    assert_eq!(first.uid_validity, Some(7));
    let second = rig
        .engine
        .headers(&a, "INBOX", first.next_before, 2)
        .await
        .unwrap();
    assert_eq!(
        second.messages.iter().map(|m| m.uid).collect::<Vec<_>>(),
        [3, 2]
    );
    let third = rig
        .engine
        .headers(&a, "INBOX", second.next_before, 2)
        .await
        .unwrap();
    assert_eq!(
        third.messages.iter().map(|m| m.uid).collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(third.next_before, None);
    let row = &first.messages[0];
    assert_eq!(row.subject, "S5");
    assert_eq!(row.from[0].email, "ann@example.com");
    assert!(!row.flags.seen);
    assert!(first.messages[1].flags.seen);
    assert_eq!(row.snippet, "body 5 needle");
    assert_eq!(row.date, Some(1_731_499_200));
    let empty = rig.engine.headers(&a, "Trash", None, 10).await.unwrap();
    assert!(empty.messages.is_empty() && empty.next_before.is_none());
}

#[tokio::test]
async fn reads_a_body_and_an_attachment_on_demand() {
    let rig = rig();
    let multipart = include_bytes!("fixtures/multipart.eml");
    let uid = rig.server.state.lock().unwrap().folders[0].add(&[], multipart);
    let a = account(true);
    let body = rig.engine.message(&a, "INBOX", uid).await.unwrap();
    assert_eq!(body.header.uid, uid);
    assert!(body.text.unwrap().contains("plain version"));
    assert!(body.html.unwrap().contains("cid:logo@example.com"));
    let file = body.attachments.iter().find(|x| !x.inline).unwrap();
    let (info, bytes) = rig
        .engine
        .attachment(&a, "INBOX", uid, file.part)
        .await
        .unwrap();
    assert_eq!(
        (info.filename.as_str(), bytes.as_slice()),
        ("report.pdf", &b"%PDF-1.4"[..])
    );
    assert_eq!(
        rig.engine.message(&a, "INBOX", 999).await.unwrap_err(),
        MailError::NotFound
    );
    assert!(
        rig.server.log().iter().any(|l| l.contains("BODY.PEEK[]")),
        "reading must not set \\Seen"
    );
}

#[tokio::test]
async fn sets_and_clears_flags() {
    let rig = rig();
    fill_inbox(&rig, 3);
    let a = account(true);
    rig.engine
        .set_flag(&a, "INBOX", &[1, 3], FlagKind::Seen, true)
        .await
        .unwrap();
    rig.engine
        .set_flag(&a, "INBOX", &[2], FlagKind::Flagged, true)
        .await
        .unwrap();
    rig.engine
        .set_flag(&a, "INBOX", &[2], FlagKind::Seen, false)
        .await
        .unwrap();
    {
        let state = rig.server.state.lock().unwrap();
        let flags = |uid: u32| {
            state.folders[0]
                .msgs
                .iter()
                .find(|m| m.uid == uid)
                .unwrap()
                .flags
                .clone()
        };
        assert!(flags(1).contains(&"\\Seen".to_owned()) && flags(3).contains(&"\\Seen".to_owned()));
        assert_eq!(flags(2), ["\\Flagged"]);
    }
    assert!(rig
        .server
        .log()
        .iter()
        .any(|l| l == "UID STORE 1,3 +FLAGS.SILENT (\\Seen)"));
    assert_eq!(
        rig.engine
            .set_flag(&a, "INBOX", &[], FlagKind::Seen, true)
            .await,
        Err(MailError::InvalidInput)
    );
}

#[tokio::test]
async fn moves_with_the_move_extension_and_deletes_into_trash_then_for_good() {
    let rig = rig();
    fill_inbox(&rig, 3);
    let a = account(true);
    rig.engine
        .move_messages(&a, "INBOX", &[1], "Archive")
        .await
        .unwrap();
    assert!(rig
        .server
        .log()
        .iter()
        .any(|l| l == "UID MOVE 1 \"Archive\""));
    rig.engine.delete(&a, "INBOX", &[2], false).await.unwrap();
    {
        let state = rig.server.state.lock().unwrap();
        assert_eq!(state.folders[4].msgs.len(), 1, "archive");
        assert_eq!(state.folders[3].msgs.len(), 1, "trash");
        assert_eq!(state.folders[0].msgs.len(), 1);
    }
    rig.engine.delete(&a, "Trash", &[1], false).await.unwrap();
    {
        let state = rig.server.state.lock().unwrap();
        assert!(state.folders[3].msgs.is_empty(), "deleted for good");
        assert!(state.log.iter().any(|l| l == "UID EXPUNGE 1"));
    }
    assert_eq!(
        rig.engine
            .move_messages(&a, "INBOX", &[3], "Nowhere")
            .await
            .unwrap_err(),
        MailError::ServerRejected
    );
}

#[tokio::test]
async fn searches_a_folder_with_quoted_text() {
    let rig = rig();
    fill_inbox(&rig, 3);
    rig.server.state.lock().unwrap().folders[0].add(
        &[],
        &simple("Bob <bob@example.com>", "Other", "nothing here"),
    );
    let hits = rig
        .engine
        .search(&account(true), "INBOX", "needle")
        .await
        .unwrap();
    assert_eq!(hits.iter().map(|m| m.uid).collect::<Vec<_>>(), [3, 2, 1]);
    assert!(rig
        .server
        .log()
        .iter()
        .any(|l| l == "UID SEARCH OR SUBJECT \"needle\" OR FROM \"needle\" TEXT \"needle\""));
    assert!(rig
        .engine
        .search(&account(true), "INBOX", "absent")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        rig.engine
            .search(&account(true), "INBOX", "x\r\ny")
            .await
            .unwrap_err(),
        MailError::InvalidInput
    );
}

fn draft() -> Draft {
    Draft {
        to: vec![addr("bob@example.com")],
        bcc: vec![addr("hidden@example.com")],
        subject: "Hello".into(),
        text: "Hi Bob".into(),
        ..Draft::default()
    }
}

#[tokio::test]
async fn sends_then_appends_to_sent_when_the_account_asks() {
    let rig = rig();
    let outcome = rig.engine.send(&account(true), &draft()).await.unwrap();
    assert!(outcome.saved_copy);
    assert_eq!(rig.smtp.sent.lock().unwrap().len(), 1);
    assert_eq!(
        rig.smtp.sent.lock().unwrap()[0].recipients,
        ["bob@example.com", "hidden@example.com"]
    );
    {
        let state = rig.server.state.lock().unwrap();
        let sent = &state.folders[1].msgs;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].flags.contains(&"\\Seen".to_owned()));
        assert!(!String::from_utf8_lossy(&sent[0].raw).contains("hidden@example.com"));
    }
    let outcome = rig.engine.send(&account(false), &draft()).await.unwrap();
    assert!(!outcome.saved_copy);
    assert_eq!(
        rig.server.state.lock().unwrap().folders[1].msgs.len(),
        1,
        "setting off: no second copy"
    );
}

#[tokio::test]
async fn a_failed_submission_stores_nothing_and_a_missing_sent_folder_does_not_fail_it() {
    let rig = rig();
    *rig.smtp.fail.lock().unwrap() = Some(MailError::ServerRejected);
    assert_eq!(
        rig.engine.send(&account(true), &draft()).await.unwrap_err(),
        MailError::ServerRejected
    );
    assert!(rig.server.state.lock().unwrap().folders[1].msgs.is_empty());
    *rig.smtp.fail.lock().unwrap() = None;
    rig.server.state.lock().unwrap().folders.remove(1);
    let outcome = rig.engine.send(&account(true), &draft()).await.unwrap();
    assert!(!outcome.saved_copy);
    let none = Draft {
        subject: "x".into(),
        ..Draft::default()
    };
    assert_eq!(
        rig.engine.send(&account(true), &none).await.unwrap_err(),
        MailError::InvalidInput
    );
}

#[tokio::test]
async fn saves_a_draft_with_its_blind_recipients() {
    let rig = rig();
    rig.engine
        .save_draft(&account(true), &draft())
        .await
        .unwrap();
    let state = rig.server.state.lock().unwrap();
    let saved = &state.folders[2].msgs;
    assert_eq!(saved.len(), 1);
    assert!(saved[0].flags.contains(&"\\Draft".to_owned()));
    assert!(String::from_utf8_lossy(&saved[0].raw).contains("hidden@example.com"));
}

#[tokio::test]
async fn bad_password_and_missing_credentials_map_to_closed_errors() {
    let rig = rig();
    let a = account(true);
    let report = rig.engine.test_account(&a, "wrong").await.unwrap();
    assert_eq!(
        (report.imap, report.smtp),
        (Some(MailError::AuthFailed), Some(MailError::AuthFailed))
    );
    let report = rig.engine.test_account(&a, PASSWORD).await.unwrap();
    assert_eq!((report.imap, report.smtp), (None, None));
    rig.vault.remove("acc-1").unwrap();
    assert_eq!(
        rig.engine.folders(&a).await.unwrap_err(),
        MailError::Credentials
    );
    assert!(
        !rig.server.log().iter().any(|l| l.contains(PASSWORD)),
        "no password in the command log"
    );
}

#[tokio::test]
async fn server_error_text_never_reaches_the_caller() {
    let rig = rig();
    rig.server.state.lock().unwrap().refuse_select = true;
    let error = rig
        .engine
        .headers(&account(true), "INBOX", None, 5)
        .await
        .unwrap_err();
    assert_eq!(error, MailError::NotFound);
    assert!(!format!("{error} {error:?}").contains("SERVER-SECRET"));
    let wrong = {
        let vault = MemoryVault::default();
        vault.set("acc-1", "wrong").unwrap();
        MailEngine::new(rig.server.clone(), Arc::new(vault), rig.smtp.clone())
    };
    let error = wrong.folders(&account(true)).await.unwrap_err();
    assert_eq!(error, MailError::AuthFailed);
    assert!(!format!("{error} {error:?}").contains("SERVER-SECRET"));
}

#[tokio::test]
async fn reuses_one_connection_and_reconnects_once_after_a_drop() {
    let rig = rig();
    fill_inbox(&rig, 2);
    let a = account(true);
    rig.engine.folders(&a).await.unwrap();
    rig.engine.headers(&a, "INBOX", None, 5).await.unwrap();
    assert_eq!(
        rig.server.connects.load(Ordering::SeqCst),
        1,
        "connection reused"
    );
    rig.server.hang_up_all();
    let page = rig.engine.headers(&a, "INBOX", None, 5).await.unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(
        rig.server.connects.load(Ordering::SeqCst),
        2,
        "reconnected exactly once"
    );
}

#[tokio::test(start_paused = true)]
async fn a_silent_server_times_out_instead_of_hanging() {
    let rig = rig();
    let a = account(true);
    rig.engine.folders(&a).await.unwrap();
    rig.server.state.lock().unwrap().silent_on = Some("EXAMINE");
    let error = rig.engine.headers(&a, "INBOX", None, 5).await.unwrap_err();
    assert_eq!(error, MailError::Timeout);
}

#[tokio::test]
async fn watching_the_inbox_reports_changes_and_honours_stop() {
    let rig = rig();
    let a = account(true);
    let stop = Notify::new();
    let waiting = rig.engine.watch_inbox(&a, &stop);
    let trigger = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        rig.server.idle.notify_one();
    };
    let (outcome, ()) = tokio::join!(waiting, trigger);
    assert_eq!(outcome.unwrap(), WatchOutcome::Changed);
    let waiting = rig.engine.watch_inbox(&a, &stop);
    let stopper = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.notify_one();
    };
    let (outcome, ()) = tokio::join!(waiting, stopper);
    assert_eq!(outcome.unwrap(), WatchOutcome::Stopped);
}

#[tokio::test]
async fn forgetting_an_account_logs_out_its_connection() {
    let rig = rig();
    let a = account(true);
    rig.engine.folders(&a).await.unwrap();
    rig.engine.forget("acc-1").await;
    assert!(rig.server.log().iter().any(|l| l == "LOGOUT"));
    rig.engine.folders(&a).await.unwrap();
    assert_eq!(rig.server.connects.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn oauth_accounts_authenticate_imap_with_xoauth2_and_never_use_the_password_vault() {
    let server = FakeConnector::new(standard_folders());
    let smtp = Arc::new(FakeSmtp::default());
    // The vault is empty: a password lookup would fail.
    let tokens = FakeTokens::new(TOKEN);
    let engine = MailEngine::new(server.clone(), Arc::new(MemoryVault::default()), smtp)
        .with_tokens(tokens.clone());
    let folders = engine.folders(&oauth_account()).await.unwrap();
    assert!(folders.iter().any(|f| f.role == FolderRole::Inbox));
    assert_eq!(tokens.asked.load(Ordering::SeqCst), 1);
    let log = server.state.lock().unwrap().log.clone();
    assert!(log.iter().any(|line| line == "AUTHENTICATE <redacted>"));
    assert!(!log.iter().any(|line| line.contains(TOKEN)));
    // The session is reused: no second token request for the next command.
    engine.folders(&oauth_account()).await.unwrap();
    assert_eq!(tokens.asked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_wrong_token_is_an_auth_failure_without_server_text() {
    let server = FakeConnector::new(standard_folders());
    let engine = MailEngine::new(
        server,
        Arc::new(MemoryVault::default()),
        Arc::new(FakeSmtp::default()),
    )
    .with_tokens(FakeTokens::new("wrong-token"));
    let error = engine.folders(&oauth_account()).await.unwrap_err();
    assert_eq!(error, MailError::AuthFailed);
    assert!(!format!("{error:?}{error}").contains("SERVER-SECRET"));
}

#[tokio::test]
async fn oauth_sends_with_the_bearer_token_and_refresh_failures_surface() {
    let server = FakeConnector::new(standard_folders());
    let smtp = Arc::new(FakeSmtp::default());
    let tokens = FakeTokens::new(TOKEN);
    let engine = MailEngine::new(server, Arc::new(MemoryVault::default()), smtp.clone())
        .with_tokens(tokens.clone());
    let draft = Draft {
        to: vec![addr("bob@example.com")],
        subject: "Hi".into(),
        text: "hello".into(),
        ..Draft::default()
    };
    engine.send(&oauth_account(), &draft).await.unwrap();
    assert_eq!(
        smtp.credentials.lock().unwrap().as_slice(),
        [format!("Credential::Bearer(..):{TOKEN}")]
    );
    // Revoked refresh token: the person must sign in again.
    *tokens.fail.lock().unwrap() = Some(zephium_oauth::OAuthError::SignInRequired);
    let error = engine.send(&oauth_account(), &draft).await.unwrap_err();
    assert_eq!(error, MailError::AuthFailed);
}

#[tokio::test]
async fn oauth_accounts_need_a_token_source() {
    let server = FakeConnector::new(standard_folders());
    let engine = MailEngine::new(
        server,
        Arc::new(MemoryVault::default()),
        Arc::new(FakeSmtp::default()),
    );
    let error = engine.folders(&oauth_account()).await.unwrap_err();
    assert_eq!(error, MailError::Credentials);
}

#[tokio::test]
async fn test_credential_accepts_a_bearer_token_before_the_account_is_saved() {
    let server = FakeConnector::new(standard_folders());
    let engine = MailEngine::new(
        server,
        Arc::new(MemoryVault::default()),
        Arc::new(FakeSmtp::default()),
    );
    let report = engine
        .test_credential(&oauth_account(), &Credential::bearer(TOKEN))
        .await
        .unwrap();
    assert_eq!((report.imap, report.smtp), (None, None));
    let report = engine
        .test_credential(&oauth_account(), &Credential::bearer("nope"))
        .await
        .unwrap();
    assert_eq!(report.imap, Some(MailError::AuthFailed));
}

#[tokio::test]
async fn rules_mark_flag_and_move_only_new_inbox_mail_of_the_account() {
    use zephium_mail::rules::{Rule, RuleAction, RuleField};
    let rig = rig();
    {
        let mut state = rig.server.state.lock().unwrap();
        for (from, subject) in [
            ("Old <old@example.com>", "News old"),
            ("Ann <ann@example.com>", "News 1"),
            ("Boss <boss@example.com>", "Plan"),
            ("Ann <ann@example.com>", "Other"),
        ] {
            state.folders[0].add(&[], &simple(from, subject, "text"));
        }
    }
    let a = account(true);
    let folders = rig.engine.folders(&a).await.unwrap();
    let rule = |field, contains: &str, action, folder: Option<&str>| Rule {
        field,
        contains: contains.into(),
        action,
        folder: folder.map(Into::into),
    };
    let rules = vec![
        rule(
            RuleField::Subject,
            "news",
            RuleAction::Move,
            Some("Archive"),
        ),
        rule(RuleField::From, "boss", RuleAction::Flag, None),
        rule(RuleField::From, "boss", RuleAction::MarkRead, None),
        rule(
            RuleField::Subject,
            "other",
            RuleAction::Move,
            Some("Nowhere"),
        ),
        rule(RuleField::Subject, "other", RuleAction::Move, Some("INBOX")),
    ];
    // UID 1 pre-dates the sync; UIDs 2..=4 are new.
    let run = rig
        .engine
        .apply_rules(&a, "INBOX", &rules, 2, &folders)
        .await
        .unwrap();
    assert_eq!(run.moved, [2]);
    assert_eq!(run.flagged, [3]);
    assert_eq!(run.seen, [3]);
    assert!(!run.failed && run.changed());
    {
        let state = rig.server.state.lock().unwrap();
        let inbox: Vec<u32> = state.folders[0].msgs.iter().map(|m| m.uid).collect();
        assert_eq!(inbox, [1, 3, 4], "only the new news mail left the inbox");
        assert_eq!(state.folders[4].msgs.len(), 1, "archive");
        let boss = state.folders[0].msgs.iter().find(|m| m.uid == 3).unwrap();
        assert!(boss.flags.contains(&"\\Seen".to_owned()));
        assert!(boss.flags.contains(&"\\Flagged".to_owned()));
    }
    // No rules, or invalid ones, touch nothing.
    let none = rig
        .engine
        .apply_rules(&a, "INBOX", &[], 1, &folders)
        .await
        .unwrap();
    assert!(!none.changed());
    let bad = vec![rule(RuleField::From, "", RuleAction::Flag, None)];
    assert_eq!(
        rig.engine.apply_rules(&a, "INBOX", &bad, 1, &folders).await,
        Err(MailError::InvalidInput)
    );
}

// Delete without a Trash folder ------------------------------------------------

/// The standard folders without the Trash one.
fn rig_without_trash() -> Rig {
    let rig = rig();
    rig.server
        .state
        .lock()
        .unwrap()
        .folders
        .retain(|folder| folder.name != "Trash");
    rig
}

#[tokio::test]
async fn deleting_without_a_trash_folder_removes_nothing_until_confirmed() {
    let rig = rig_without_trash();
    fill_inbox(&rig, 2);
    let a = account(false);
    assert_eq!(
        rig.engine.delete(&a, "INBOX", &[1], false).await,
        Err(MailError::NoTrash)
    );
    {
        let state = rig.server.state.lock().unwrap();
        assert_eq!(state.folders[0].msgs.len(), 2, "nothing was removed");
        assert!(
            !state
                .log
                .iter()
                .any(|line| line.contains("STORE") || line.contains("EXPUNGE")),
            "no flag change and no expunge was sent"
        );
    }
    rig.engine.delete(&a, "INBOX", &[1], true).await.unwrap();
    let state = rig.server.state.lock().unwrap();
    assert_eq!(state.folders[0].msgs.len(), 1, "removed after confirmation");
    assert!(state.log.iter().any(|line| line == "UID EXPUNGE 1"));
}

#[tokio::test]
async fn a_trash_folder_with_a_local_name_is_found_without_special_use() {
    let rig = rig();
    fill_inbox(&rig, 1);
    {
        let mut state = rig.server.state.lock().unwrap();
        // Same folder, German name, no SPECIAL-USE attribute.
        let trash = state
            .folders
            .iter_mut()
            .find(|folder| folder.name == "Trash")
            .unwrap();
        trash.name = "Papierkorb";
        trash.wire = "Papierkorb";
        trash.attrs = r"\HasNoChildren";
    }
    let a = account(false);
    rig.engine.delete(&a, "INBOX", &[1], false).await.unwrap();
    let state = rig.server.state.lock().unwrap();
    assert_eq!(state.folders[3].msgs.len(), 1, "moved into Papierkorb");
}

// A hostile server -------------------------------------------------------------

#[tokio::test]
async fn a_literal_larger_than_promised_is_cut_off_and_the_connection_dropped() {
    let rig = rig();
    rig.server.state.lock().unwrap().folders[0].add(&[], &simple("a@example.com", "x", "tiny"));
    rig.server.state.lock().unwrap().hostile = Hostile::LyingSize;
    let a = account(false);
    assert_eq!(
        rig.engine.message(&a, "INBOX", 1).await.unwrap_err(),
        MailError::TooLarge
    );
    // The poisoned connection was not reused: the next call connects afresh.
    rig.server.state.lock().unwrap().hostile = Hostile::None;
    let before = rig.server.connects.load(Ordering::SeqCst);
    let body = rig.engine.message(&a, "INBOX", 1).await.unwrap();
    assert!(body.text.unwrap_or_default().contains("tiny"));
    assert_eq!(rig.server.connects.load(Ordering::SeqCst), before + 1);
}

#[tokio::test]
async fn a_folder_list_that_never_ends_stops_at_its_cap() {
    let rig = rig();
    rig.server.state.lock().unwrap().hostile = Hostile::EndlessList;
    let a = account(false);
    let started = std::time::Instant::now();
    assert_eq!(
        rig.engine.folders(&a).await.unwrap_err(),
        MailError::Protocol
    );
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn one_endless_line_stops_at_the_byte_budget() {
    let rig = rig();
    rig.server.state.lock().unwrap().hostile = Hostile::EndlessLine;
    let a = account(false);
    let started = std::time::Instant::now();
    // The client library reports the refused read as a dead or broken
    // connection; either way the call ends, quickly, with a connection error.
    let error = rig.engine.folders(&a).await.unwrap_err();
    assert!(error.is_connection(), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn a_header_fetch_that_never_ends_stops_at_its_cap() {
    let rig = rig();
    fill_inbox(&rig, 2);
    rig.server.state.lock().unwrap().hostile = Hostile::EndlessRows;
    let a = account(false);
    assert_eq!(
        rig.engine.headers(&a, "INBOX", None, 50).await.unwrap_err(),
        MailError::Protocol
    );
}

#[tokio::test]
async fn a_huge_announced_literal_is_refused_before_it_is_allocated_or_awaited() {
    let rig = rig();
    rig.server.state.lock().unwrap().folders[0].add(&[], &simple("a@example.com", "x", "tiny"));
    rig.server.state.lock().unwrap().hostile = Hostile::HugeAnnounce;
    let a = account(false);
    let started = std::time::Instant::now();
    assert_eq!(
        rig.engine.message(&a, "INBOX", 1).await.unwrap_err(),
        MailError::TooLarge
    );
    // Refused on the announcement, not after a timeout waiting for the bytes.
    assert!(started.elapsed() < Duration::from_secs(10));
}
