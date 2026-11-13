//! Composing and submitting mail (T-0029). Messages are built with
//! `mail-builder` (which encodes every header) after each address, id and
//! size has been validated here; `lettre` only carries the bytes over SMTP.

use futures::future::BoxFuture;
use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use mail_builder::MessageBuilder;

use crate::auth::Credential;
use crate::error::{MailError, Result};
use crate::limits::{
    MAX_OUTGOING_BYTES, MAX_RECIPIENTS, MAX_REFERENCES, MAX_SUBJECT_BYTES, SEND_TIMEOUT,
};
use crate::mime::sanitize_filename;
use crate::model::{email_ok, Account, Address, Draft, Endpoint, Security};

/// A composed message ready for SMTP or for APPEND.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Composed {
    /// The envelope sender.
    pub sender: String,
    /// Every recipient, including Bcc.
    pub recipients: Vec<String>,
    /// The RFC 5322 bytes.
    pub raw: Vec<u8>,
    /// The Message-ID without brackets.
    pub message_id: String,
}

fn message_id_ok(id: &str) -> bool {
    (1..=998).contains(&id.len())
        && !id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>'))
}

fn mailbox(address: &Address) -> Result<(String, String)> {
    if !email_ok(&address.email) {
        return Err(MailError::InvalidInput);
    }
    let name = address.name.clone().unwrap_or_default();
    if name.len() > 256 || name.chars().any(char::is_control) {
        return Err(MailError::InvalidInput);
    }
    Ok((name, address.email.clone()))
}

fn mailboxes(list: &[Address]) -> Result<Vec<(String, String)>> {
    list.iter().map(mailbox).collect()
}

/// Builds the message. `include_bcc` is true only for a draft kept on the
/// server; a message in transit never names its blind recipients.
pub fn compose(account: &Account, draft: &Draft, include_bcc: bool, now: i64) -> Result<Composed> {
    account.validate()?;
    let recipient_count = draft.to.len() + draft.cc.len() + draft.bcc.len();
    if recipient_count > MAX_RECIPIENTS || draft.subject.len() > MAX_SUBJECT_BYTES {
        return Err(MailError::InvalidInput);
    }
    if draft.subject.contains(['\r', '\n']) {
        return Err(MailError::InvalidInput);
    }
    if let Some(id) = &draft.in_reply_to {
        if !message_id_ok(id) {
            return Err(MailError::InvalidInput);
        }
    }
    if draft.references.len() > MAX_REFERENCES
        || !draft.references.iter().all(|id| message_id_ok(id))
    {
        return Err(MailError::InvalidInput);
    }
    let to = mailboxes(&draft.to)?;
    let cc = mailboxes(&draft.cc)?;
    let bcc = mailboxes(&draft.bcc)?;
    let message_id = format!(
        "{}@{}",
        ulid::Ulid::new().to_string().to_ascii_lowercase(),
        account
            .email
            .rsplit_once('@')
            .map_or("localhost", |(_, domain)| domain)
    );
    let mut builder = MessageBuilder::new()
        .from((account.display_name.clone(), account.email.clone()))
        .subject(draft.subject.clone())
        .date(now)
        .message_id(message_id.clone())
        .text_body(draft.text.clone());
    if !to.is_empty() {
        builder = builder.to(to.clone());
    }
    if !cc.is_empty() {
        builder = builder.cc(cc.clone());
    }
    if include_bcc && !bcc.is_empty() {
        builder = builder.bcc(bcc.clone());
    }
    if let Some(html) = &draft.html {
        builder = builder.html_body(html.clone());
    }
    if let Some(id) = &draft.in_reply_to {
        builder = builder.in_reply_to(id.clone());
    }
    if !draft.references.is_empty() {
        builder = builder.references(draft.references.clone());
    }
    let mut attachment_bytes = 0usize;
    for attachment in &draft.attachments {
        attachment_bytes = attachment_bytes.saturating_add(attachment.data.len());
        if attachment_bytes > MAX_OUTGOING_BYTES
            || attachment.content_type.len() > 127
            || attachment
                .content_type
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || !attachment.content_type.contains('/')
        {
            return Err(MailError::TooLarge);
        }
        builder = builder.attachment(
            attachment.content_type.clone(),
            sanitize_filename(&attachment.filename),
            attachment.data.clone(),
        );
    }
    let raw = builder.write_to_vec().map_err(|_| MailError::Internal)?;
    if raw.len() > MAX_OUTGOING_BYTES {
        return Err(MailError::TooLarge);
    }
    let recipients: Vec<String> = to
        .iter()
        .chain(&cc)
        .chain(&bcc)
        .map(|(_, email)| email.clone())
        .collect();
    Ok(Composed {
        sender: account.email.clone(),
        recipients,
        raw,
        message_id,
    })
}

/// Submits a composed message.
pub trait SmtpSender: Send + Sync {
    /// Sends `composed` through the account's outgoing server.
    fn send<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        username: &'a str,
        credential: &'a Credential,
        composed: &'a Composed,
    ) -> BoxFuture<'a, Result<()>>;

    /// Connects and authenticates without sending.
    fn check<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        username: &'a str,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<()>>;
}

/// The production sender, on `lettre` with rustls and the platform verifier.
#[derive(Clone, Copy, Debug, Default)]
pub struct LettreSender;

fn transport(
    endpoint: &Endpoint,
    username: &str,
    credential: &Credential,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    endpoint.validate()?;
    let tls = TlsParameters::builder(endpoint.host.clone())
        .build_rustls()
        .map_err(|_| MailError::Tls)?;
    let builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(endpoint.host.clone())
        .port(endpoint.port)
        .tls(match endpoint.security {
            Security::Tls => Tls::Wrapper(tls),
            Security::StartTls => Tls::Required(tls),
        })
        .credentials(Credentials::new(
            username.to_owned(),
            credential.secret().to_owned(),
        ))
        .timeout(Some(SEND_TIMEOUT));
    Ok(match credential {
        // lettre builds the XOAUTH2 initial response from user and token.
        Credential::Bearer(_) => builder.authentication(vec![Mechanism::Xoauth2]),
        Credential::Password(_) => builder,
    }
    .build())
}

fn map_smtp(error: &lettre::transport::smtp::Error) -> MailError {
    if error.is_permanent() {
        // 535 and friends arrive as permanent failures during AUTH.
        if error.is_response() {
            return MailError::AuthFailed;
        }
        return MailError::ServerRejected;
    }
    if error.is_timeout() {
        MailError::Timeout
    } else if error.is_tls() {
        MailError::Tls
    } else if error.is_transient() {
        MailError::ServerRejected
    } else {
        MailError::Offline
    }
}

impl SmtpSender for LettreSender {
    fn send<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        username: &'a str,
        credential: &'a Credential,
        composed: &'a Composed,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let transport = transport(endpoint, username, credential)?;
            let sender = composed
                .sender
                .parse()
                .map_err(|_| MailError::InvalidInput)?;
            let recipients = composed
                .recipients
                .iter()
                .map(|email| email.parse().map_err(|_| MailError::InvalidInput))
                .collect::<Result<Vec<_>>>()?;
            let envelope =
                Envelope::new(Some(sender), recipients).map_err(|_| MailError::InvalidInput)?;
            tokio::time::timeout(SEND_TIMEOUT, transport.send_raw(&envelope, &composed.raw))
                .await?
                .map(|_| ())
                .map_err(|error| map_smtp(&error))
        })
    }

    fn check<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        username: &'a str,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let transport = transport(endpoint, username, credential)?;
            match tokio::time::timeout(SEND_TIMEOUT, transport.test_connection()).await? {
                Ok(true) => Ok(()),
                Ok(false) => Err(MailError::Offline),
                Err(error) => Err(map_smtp(&error)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OutgoingAttachment, Security};

    fn account() -> Account {
        let endpoint = |port| Endpoint {
            host: "mail.example.com".into(),
            port,
            security: Security::Tls,
        };
        Account {
            id: "a1".into(),
            display_name: "Ann Äpfel".into(),
            email: "ann@example.com".into(),
            imap: endpoint(993),
            smtp: endpoint(465),
            username: "ann".into(),
            colour: "#336699".into(),
            signature: String::new(),
            save_sent_copy: true,
            auth: crate::auth::AuthKind::Password,
        }
    }

    fn address(email: &str) -> Address {
        Address {
            name: None,
            email: email.into(),
        }
    }

    fn draft() -> Draft {
        Draft {
            to: vec![address("bob@example.com")],
            cc: vec![address("cc@example.com")],
            bcc: vec![address("secret@example.com")],
            subject: "Grüße".into(),
            text: "Hallo".into(),
            html: Some("<p>Hallo</p>".into()),
            attachments: vec![OutgoingAttachment {
                filename: "../a.txt".into(),
                content_type: "text/plain".into(),
                data: b"data".to_vec(),
            }],
            in_reply_to: Some("m0@example.com".into()),
            references: vec!["m-1@example.com".into(), "m0@example.com".into()],
        }
    }

    #[test]
    fn composed_message_round_trips_through_the_parser_and_hides_bcc() {
        let composed = compose(&account(), &draft(), false, 1_731_499_200).unwrap();
        assert_eq!(composed.sender, "ann@example.com");
        assert_eq!(
            composed.recipients,
            ["bob@example.com", "cc@example.com", "secret@example.com"]
        );
        let raw = String::from_utf8_lossy(&composed.raw).to_lowercase();
        assert!(!raw.contains("secret@example.com"));
        assert!(!raw.contains("\r\nbcc:"));
        let parsed = crate::mime::parse_message(&composed.raw).unwrap();
        assert_eq!(parsed.header.subject, "Grüße");
        assert_eq!(parsed.header.from[0].email, "ann@example.com");
        assert_eq!(parsed.header.from[0].name.as_deref(), Some("Ann Äpfel"));
        assert_eq!(parsed.header.in_reply_to.as_deref(), Some("m0@example.com"));
        assert_eq!(
            parsed.header.references,
            ["m-1@example.com", "m0@example.com"]
        );
        assert_eq!(parsed.header.date, Some(1_731_499_200));
        assert_eq!(
            parsed.header.message_id.as_deref(),
            Some(composed.message_id.as_str())
        );
        assert_eq!(parsed.text.as_deref().map(str::trim), Some("Hallo"));
        assert!(parsed.html.as_deref().unwrap().contains("<p>Hallo</p>"));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].filename, "_a.txt");
    }

    #[test]
    fn a_draft_on_the_server_keeps_its_bcc() {
        let composed = compose(&account(), &draft(), true, 0).unwrap();
        assert!(String::from_utf8_lossy(&composed.raw).contains("secret@example.com"));
    }

    #[test]
    fn rejects_injection_and_oversized_input() {
        let mut bad = draft();
        bad.subject = "x\r\nBcc: evil@example.com".into();
        assert_eq!(
            compose(&account(), &bad, false, 0),
            Err(MailError::InvalidInput)
        );
        let mut bad = draft();
        bad.to.push(address("a@b.com>\r\nRCPT TO:<x@y.com"));
        assert_eq!(
            compose(&account(), &bad, false, 0),
            Err(MailError::InvalidInput)
        );
        let mut bad = draft();
        bad.in_reply_to = Some("a b".into());
        assert_eq!(
            compose(&account(), &bad, false, 0),
            Err(MailError::InvalidInput)
        );
        let mut bad = draft();
        bad.to = (0..MAX_RECIPIENTS + 1)
            .map(|i| address(&format!("u{i}@example.com")))
            .collect();
        assert_eq!(
            compose(&account(), &bad, false, 0),
            Err(MailError::InvalidInput)
        );
        let mut bad = draft();
        bad.attachments[0].data = vec![0; MAX_OUTGOING_BYTES + 1];
        assert_eq!(
            compose(&account(), &bad, false, 0),
            Err(MailError::TooLarge)
        );
    }
}
