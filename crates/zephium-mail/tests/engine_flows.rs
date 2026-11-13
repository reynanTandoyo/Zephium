//! Protocol flows through the real engine against the in-process fake server.

mod fake_imap;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fake_imap::{simple, standard_folders, FakeConnector, PASSWORD};
use futures::future::BoxFuture;
use tokio::sync::Notify;
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
    fail: Mutex<Option<MailError>>,
}

impl SmtpSender for FakeSmtp {
    fn send<'a>(
        &'a self,
        _e: &'a Endpoint,
        _u: &'a str,
        password: &'a str,
        composed: &'a Composed,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            assert_eq!(password, PASSWORD);
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
        password: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if password == PASSWORD {
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
    rig.engine.delete(&a, "INBOX", &[2]).await.unwrap();
    {
        let state = rig.server.state.lock().unwrap();
        assert_eq!(state.folders[4].msgs.len(), 1, "archive");
        assert_eq!(state.folders[3].msgs.len(), 1, "trash");
        assert_eq!(state.folders[0].msgs.len(), 1);
    }
    rig.engine.delete(&a, "Trash", &[1]).await.unwrap();
    let state = rig.server.state.lock().unwrap();
    assert!(state.folders[3].msgs.is_empty(), "deleted for good");
    assert!(state.log.iter().any(|l| l == "UID EXPUNGE 1"));
    drop(state);
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
