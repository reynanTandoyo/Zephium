//! Fork-owned (T-0036): simple per-account rules. "If From/To/Subject
//! contains X, then move / mark read / flag", applied to new Inbox mail after
//! a sync. Pure data and matching: the caller reads headers of one account's
//! Inbox and runs the plan; a rule only ever sees that account's message
//! headers, never a body or another account.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::engine::MailEngine;
use crate::error::{MailError, Result};
use crate::model::{Account, Address, FlagKind, Folder, MessageHeader};

/// Rules kept per account.
pub const MAX_RULES_PER_ACCOUNT: usize = 30;
/// Bytes of the text a rule looks for.
pub const MAX_NEEDLE_BYTES: usize = 120;
/// Bytes of a rule's destination folder path.
pub const MAX_RULE_FOLDER_BYTES: usize = 256;
/// Newest Inbox messages one run looks at.
pub const MAX_RUN_MESSAGES: u32 = 50;
/// Bytes of the stored rule set for every account together; the app-setting
/// store refuses values above 64 KiB.
pub const MAX_RULES_BYTES: usize = 56 * 1024;

/// The header a rule looks at.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleField {
    /// The sender's name or address.
    From,
    /// A recipient's name or address (To and Cc).
    To,
    /// The subject.
    Subject,
}

/// What a matching rule does.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    /// Moves the message to the rule's folder.
    Move,
    /// Marks it read.
    MarkRead,
    /// Flags it.
    Flag,
}

/// One rule. `folder` is required for [`RuleAction::Move`] and absent otherwise.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// Header to look at.
    pub field: RuleField,
    /// Case-insensitive text the header must contain.
    pub contains: String,
    /// What to do.
    pub action: RuleAction,
    /// Destination folder path for a move.
    #[serde(default)]
    pub folder: Option<String>,
}

/// Every account's rules, by account id.
pub type RuleSet = BTreeMap<String, Vec<Rule>>;

/// What the rules decided for one message.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct Plan {
    /// Mark the message read.
    pub mark_read: bool,
    /// Flag the message.
    pub flag: bool,
    /// Move the message here (the first matching move rule wins).
    pub move_to: Option<String>,
}

impl Plan {
    /// True when nothing is to be done.
    pub fn is_empty(&self) -> bool {
        !self.mark_read && !self.flag && self.move_to.is_none()
    }
}

fn clean_text(text: &str) -> bool {
    !text.chars().any(char::is_control)
}

impl Rule {
    /// Checks the rule against its bounds.
    pub fn validate(&self) -> Result<()> {
        let needle = self.contains.trim();
        if needle.is_empty() || self.contains.len() > MAX_NEEDLE_BYTES || !clean_text(needle) {
            return Err(MailError::InvalidInput);
        }
        match (self.action, self.folder.as_deref()) {
            (RuleAction::Move, Some(folder)) => {
                if folder.is_empty() || folder.len() > MAX_RULE_FOLDER_BYTES || !clean_text(folder)
                {
                    return Err(MailError::InvalidInput);
                }
            }
            (RuleAction::Move, None) => return Err(MailError::InvalidInput),
            (_, Some(_)) => return Err(MailError::InvalidInput),
            (_, None) => {}
        }
        Ok(())
    }

    /// True when the message's header contains the rule's text.
    pub fn matches(&self, header: &MessageHeader) -> bool {
        let needle = self.contains.trim().to_lowercase();
        if needle.is_empty() {
            return false;
        }
        let has = |text: &str| text.to_lowercase().contains(&needle);
        let any = |list: &[Address]| {
            list.iter()
                .any(|address| has(&address.email) || address.name.as_deref().is_some_and(has))
        };
        match self.field {
            RuleField::From => any(&header.from),
            RuleField::To => any(&header.to) || any(&header.cc),
            RuleField::Subject => has(&header.subject),
        }
    }
}

/// Validates one account's rules.
pub fn validate_rules(rules: &[Rule]) -> Result<()> {
    if rules.len() > MAX_RULES_PER_ACCOUNT {
        return Err(MailError::TooLarge);
    }
    rules.iter().try_for_each(Rule::validate)
}

/// Parses the stored rule set. Anything unreadable, over a bound or invalid
/// is dropped as a whole account's list, never half-applied.
pub fn parse_rule_set(json: &str) -> RuleSet {
    if json.len() > MAX_RULES_BYTES {
        return RuleSet::new();
    }
    let Ok(set) = serde_json::from_str::<RuleSet>(json) else {
        return RuleSet::new();
    };
    set.into_iter()
        .filter(|(id, rules)| crate::model::id_ok(id) && validate_rules(rules).is_ok())
        .collect()
}

/// Serializes a rule set, refusing one over its bounds.
pub fn serialize_rule_set(set: &RuleSet) -> Result<String> {
    for (id, rules) in set {
        if !crate::model::id_ok(id) {
            return Err(MailError::InvalidInput);
        }
        validate_rules(rules)?;
    }
    let json = serde_json::to_string(set).map_err(|_| MailError::Internal)?;
    if json.len() > MAX_RULES_BYTES {
        return Err(MailError::TooLarge);
    }
    Ok(json)
}

/// Runs the rules in order over one message. Marking and flagging add up; the
/// first move wins and ends the run, since the message leaves the folder.
pub fn plan(rules: &[Rule], header: &MessageHeader) -> Plan {
    plan_with(rules, header, |_| true)
}

/// [`plan`], where a move to a folder `can_move_to` refuses is skipped and the
/// rules after it still run.
pub fn plan_with(
    rules: &[Rule],
    header: &MessageHeader,
    can_move_to: impl Fn(&str) -> bool,
) -> Plan {
    let mut plan = Plan::default();
    for rule in rules {
        if !rule.matches(header) {
            continue;
        }
        match rule.action {
            RuleAction::MarkRead => plan.mark_read = true,
            RuleAction::Flag => plan.flag = true,
            RuleAction::Move => {
                let Some(folder) = rule.folder.as_deref().filter(|folder| can_move_to(folder))
                else {
                    continue;
                };
                plan.move_to = Some(folder.to_owned());
                break;
            }
        }
    }
    plan
}

/// What the rules want done to a batch of messages, grouped by command.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct Batch {
    /// UIDs to mark read.
    pub seen: Vec<u32>,
    /// UIDs to flag.
    pub flagged: Vec<u32>,
    /// Destination folder and the UIDs to move there, in first-use order.
    pub moves: Vec<(String, Vec<u32>)>,
}

impl Batch {
    /// True when nothing is to be done.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty() && self.flagged.is_empty() && self.moves.is_empty()
    }
}

/// Plans every header at or above `min_uid`. `can_move_to` says which
/// destinations exist; a rule that names any other folder only skips its move.
pub fn plan_batch(
    rules: &[Rule],
    headers: &[MessageHeader],
    min_uid: u32,
    can_move_to: impl Fn(&str) -> bool,
) -> Batch {
    let mut batch = Batch::default();
    for header in headers.iter().filter(|header| header.uid >= min_uid) {
        let planned = plan_with(rules, header, &can_move_to);
        if planned.mark_read && !header.flags.seen {
            batch.seen.push(header.uid);
        }
        if planned.flag && !header.flags.flagged {
            batch.flagged.push(header.uid);
        }
        if let Some(dest) = planned.move_to {
            match batch.moves.iter_mut().find(|(known, _)| *known == dest) {
                Some((_, uids)) => uids.push(header.uid),
                None => batch.moves.push((dest, vec![header.uid])),
            }
        }
    }
    batch
}

/// What a rule run did on the server. A list holds only what the server accepted.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RuleRun {
    /// UIDs marked read.
    pub seen: Vec<u32>,
    /// UIDs flagged.
    pub flagged: Vec<u32>,
    /// UIDs moved out of the Inbox.
    pub moved: Vec<u32>,
    /// A command failed; the rest still ran.
    pub failed: bool,
}

impl RuleRun {
    /// True when the server changed anything.
    pub fn changed(&self) -> bool {
        !self.seen.is_empty() || !self.flagged.is_empty() || !self.moved.is_empty()
    }
}

impl MailEngine {
    /// Applies one account's `rules` to its newest Inbox messages with a UID
    /// of `min_uid` or more (the ones a sync just found). Reads only that
    /// account's Inbox headers; moves go only to folders in `folders`, never
    /// to the Inbox itself. A failing command does not stop the others.
    pub async fn apply_rules(
        &self,
        account: &Account,
        inbox: &str,
        rules: &[Rule],
        min_uid: u32,
        folders: &[Folder],
    ) -> Result<RuleRun> {
        validate_rules(rules)?;
        if rules.is_empty() {
            return Ok(RuleRun::default());
        }
        let page = self.headers(account, inbox, None, MAX_RUN_MESSAGES).await?;
        let batch = plan_batch(rules, &page.messages, min_uid, |dest| {
            dest != inbox
                && folders
                    .iter()
                    .any(|folder| folder.path == dest && folder.selectable)
        });
        let mut run = RuleRun::default();
        if !batch.seen.is_empty() {
            match self
                .set_flag(account, inbox, &batch.seen, FlagKind::Seen, true)
                .await
            {
                Ok(()) => run.seen = batch.seen,
                Err(_) => run.failed = true,
            }
        }
        if !batch.flagged.is_empty() {
            match self
                .set_flag(account, inbox, &batch.flagged, FlagKind::Flagged, true)
                .await
            {
                Ok(()) => run.flagged = batch.flagged,
                Err(_) => run.failed = true,
            }
        }
        for (dest, uids) in batch.moves {
            match self.move_messages(account, inbox, &uids, &dest).await {
                Ok(()) => run.moved.extend(uids),
                Err(_) => run.failed = true,
            }
        }
        Ok(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> MessageHeader {
        MessageHeader {
            uid: 9,
            subject: "Your INVOICE for May".into(),
            from: vec![Address {
                name: Some("Billing Team".into()),
                email: "billing@shop.example".into(),
            }],
            to: vec![Address {
                name: None,
                email: "me@example.com".into(),
            }],
            cc: vec![Address {
                name: None,
                email: "boss@example.com".into(),
            }],
            ..MessageHeader::default()
        }
    }

    fn rule(field: RuleField, contains: &str, action: RuleAction, folder: Option<&str>) -> Rule {
        Rule {
            field,
            contains: contains.into(),
            action,
            folder: folder.map(Into::into),
        }
    }

    #[test]
    fn matching_is_case_insensitive_and_looks_at_names_and_addresses() {
        let h = header();
        assert!(rule(RuleField::From, "billing team", RuleAction::Flag, None).matches(&h));
        assert!(rule(RuleField::From, "SHOP.EXAMPLE", RuleAction::Flag, None).matches(&h));
        assert!(rule(RuleField::Subject, "invoice", RuleAction::Flag, None).matches(&h));
        assert!(rule(RuleField::To, "boss@", RuleAction::Flag, None).matches(&h));
        assert!(!rule(RuleField::To, "billing", RuleAction::Flag, None).matches(&h));
        assert!(!rule(RuleField::Subject, "   ", RuleAction::Flag, None).matches(&h));
    }

    #[test]
    fn rules_apply_in_order_and_the_first_move_ends_the_run() {
        let rules = vec![
            rule(RuleField::Subject, "invoice", RuleAction::MarkRead, None),
            rule(RuleField::From, "billing", RuleAction::Move, Some("Bills")),
            rule(RuleField::Subject, "may", RuleAction::Move, Some("Later")),
            rule(RuleField::Subject, "invoice", RuleAction::Flag, None),
        ];
        let planned = plan(&rules, &header());
        assert_eq!(
            planned,
            Plan {
                mark_read: true,
                flag: false,
                move_to: Some("Bills".into())
            }
        );
        assert!(plan(&rules, &MessageHeader::default()).is_empty());
    }

    #[test]
    fn a_batch_groups_by_command_skips_old_uids_and_unknown_folders() {
        let at = |uid: u32, subject: &str, seen: bool| MessageHeader {
            uid,
            subject: subject.into(),
            flags: crate::model::Flags {
                seen,
                ..Default::default()
            },
            ..header()
        };
        let rules = vec![
            rule(RuleField::Subject, "news", RuleAction::MarkRead, None),
            rule(RuleField::Subject, "news", RuleAction::Move, Some("News")),
            rule(
                RuleField::Subject,
                "ghost",
                RuleAction::Move,
                Some("Missing"),
            ),
            rule(RuleField::Subject, "ghost", RuleAction::Flag, None),
        ];
        let headers = [
            at(9, "news a", false),
            at(8, "news b", true),
            at(7, "ghost", false),
            at(2, "news old", false),
        ];
        let batch = plan_batch(&rules, &headers, 5, |dest| dest == "News");
        assert_eq!(
            batch,
            Batch {
                seen: vec![9],
                flagged: vec![7],
                moves: vec![("News".into(), vec![9, 8])],
            }
        );
        assert!(plan_batch(&[], &headers, 0, |_| true).is_empty());
    }

    #[test]
    fn rules_are_validated_and_bounded() {
        assert!(rule(RuleField::From, "a", RuleAction::Move, Some("Bills"))
            .validate()
            .is_ok());
        let bad = [
            rule(RuleField::From, "", RuleAction::Flag, None),
            rule(RuleField::From, "  ", RuleAction::Flag, None),
            rule(RuleField::From, "a\r\nBcc: x", RuleAction::Flag, None),
            rule(
                RuleField::From,
                &"x".repeat(MAX_NEEDLE_BYTES + 1),
                RuleAction::Flag,
                None,
            ),
            rule(RuleField::From, "a", RuleAction::Move, None),
            rule(RuleField::From, "a", RuleAction::Move, Some("")),
            rule(RuleField::From, "a", RuleAction::Move, Some("A\nB")),
            rule(RuleField::From, "a", RuleAction::Flag, Some("Bills")),
            rule(
                RuleField::From,
                "a",
                RuleAction::Move,
                Some(&"f".repeat(MAX_RULE_FOLDER_BYTES + 1)),
            ),
        ];
        for rule in bad {
            assert_eq!(rule.validate(), Err(MailError::InvalidInput), "{rule:?}");
        }
        let many =
            vec![rule(RuleField::From, "a", RuleAction::Flag, None); MAX_RULES_PER_ACCOUNT + 1];
        assert_eq!(validate_rules(&many), Err(MailError::TooLarge));
    }

    #[test]
    fn the_stored_set_round_trips_and_a_bad_account_is_dropped() {
        let mut set = RuleSet::new();
        set.insert(
            "acc-1".into(),
            vec![rule(RuleField::From, "boss", RuleAction::Flag, None)],
        );
        let json = serialize_rule_set(&set).unwrap();
        assert_eq!(parse_rule_set(&json), set);

        let tampered = r#"{"acc-1":[{"field":"from","contains":"","action":"flag"}],
            "bad id!":[],"acc-2":[{"field":"subject","contains":"x","action":"mark_read"}]}"#;
        let parsed = parse_rule_set(tampered);
        assert_eq!(parsed.keys().collect::<Vec<_>>(), vec!["acc-2"]);
        assert!(parse_rule_set("not json").is_empty());
        assert!(parse_rule_set(&" ".repeat(MAX_RULES_BYTES + 1)).is_empty());
    }

    #[test]
    fn a_set_over_the_size_bound_is_refused() {
        let mut set = RuleSet::new();
        let full = vec![
            rule(
                RuleField::From,
                &"n".repeat(MAX_NEEDLE_BYTES),
                RuleAction::Move,
                Some(&"f".repeat(MAX_RULE_FOLDER_BYTES)),
            );
            MAX_RULES_PER_ACCOUNT
        ];
        for index in 0..16 {
            set.insert(format!("acc-{index}"), full.clone());
        }
        assert_eq!(serialize_rule_set(&set), Err(MailError::TooLarge));
    }
}
