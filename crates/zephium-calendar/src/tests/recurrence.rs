use crate::api::CalendarRecurrenceV1;
use crate::error::CalendarError;
use crate::model::{EventTime, TimeKind};
use crate::recurrence::{build, describe};
use chrono::NaiveDate;

fn zoned() -> EventTime {
    EventTime {
        kind: TimeKind::Zoned(chrono_tz::Europe::Berlin),
        local: NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap(),
    }
}

fn simple(freq: &str) -> CalendarRecurrenceV1 {
    CalendarRecurrenceV1 {
        freq: freq.to_owned(),
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        custom: false,
    }
}

#[test]
fn simple_rules_are_described_and_rebuilt_unchanged() {
    for rule in [
        "FREQ=DAILY",
        "FREQ=DAILY;COUNT=5",
        "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10",
        "FREQ=MONTHLY;BYDAY=2TU",
        "FREQ=MONTHLY;BYDAY=-1FR;UNTIL=20261231T225959Z",
        "FREQ=MONTHLY;BYMONTHDAY=1,15,-1",
        "FREQ=YEARLY;BYMONTH=3,9",
    ] {
        let described = describe(rule, &zoned()).unwrap_or_else(|| panic!("{rule}"));
        assert!(!described.custom, "{rule}");
        assert_eq!(build(&described, &zoned()).expect("rebuilt"), rule);
    }
}

#[test]
fn a_description_carries_every_simple_part() {
    let rule = describe("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10", &zoned()).expect("rule");
    assert_eq!(rule.freq, "weekly");
    assert_eq!(rule.interval, 2);
    assert_eq!(rule.count, Some(10));
    assert_eq!(rule.until, None);
    assert_eq!(rule.by_day, ["MO", "WE"]);

    let rule = describe("FREQ=DAILY;UNTIL=20260301", &zoned()).expect("rule");
    assert_eq!(rule.until.as_deref(), Some("2026-03-01"));
    let rule = describe("FREQ=DAILY;UNTIL=20260301T120000Z", &zoned()).expect("rule");
    assert_eq!(rule.until.as_deref(), Some("2026-03-01T12:00:00Z"));
    let all_day = EventTime::date(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap());
    let rule = describe("FREQ=DAILY;UNTIL=20260301T000000Z", &all_day).expect("rule");
    assert_eq!(rule.until.as_deref(), Some("2026-03-01"));
}

#[test]
fn rules_beyond_the_description_are_flagged_or_left_out() {
    let rule = describe("FREQ=MONTHLY;BYDAY=MO,TU;BYSETPOS=-1", &zoned()).expect("rule");
    assert!(rule.custom);
    assert_eq!(rule.by_day, ["MO", "TU"]);
    assert!(
        !describe("FREQ=WEEKLY;WKST=MO", &zoned())
            .expect("rule")
            .custom
    );
    assert!(
        describe("FREQ=WEEKLY;WKST=SU", &zoned())
            .expect("rule")
            .custom
    );
    assert_eq!(describe("FREQ=HOURLY", &zoned()), None);
    assert_eq!(describe("INTERVAL=2", &zoned()), None);
    assert_eq!(describe("FREQ=DAILY;INTERVAL=0", &zoned()), None);
    assert_eq!(describe("FREQ=WEEKLY;BYDAY=XX", &zoned()), None);
    assert_eq!(describe("FREQ=DAILY;COUNT=many", &zoned()), None);
    assert_eq!(describe("nonsense", &zoned()), None);
}

#[test]
fn until_is_written_the_way_the_start_requires() {
    let mut rule = simple("daily");
    rule.until = Some("2026-03-01".to_owned());
    // Zoned: the whole last day counts, as a UTC instant (CET is UTC+1).
    assert_eq!(
        build(&rule, &zoned()).expect("zoned"),
        "FREQ=DAILY;UNTIL=20260301T225959Z"
    );
    let floating = EventTime {
        kind: TimeKind::Floating,
        local: zoned().local,
    };
    assert_eq!(
        build(&rule, &floating).expect("floating"),
        "FREQ=DAILY;UNTIL=20260301T235959"
    );
    let all_day = EventTime::date(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap());
    assert_eq!(
        build(&rule, &all_day).expect("date"),
        "FREQ=DAILY;UNTIL=20260301"
    );
    let utc = EventTime {
        kind: TimeKind::Utc,
        local: zoned().local,
    };
    rule.until = Some("2026-03-01T12:00:00Z".to_owned());
    assert_eq!(
        build(&rule, &utc).expect("utc"),
        "FREQ=DAILY;UNTIL=20260301T120000Z"
    );
    assert_eq!(
        build(&rule, &zoned()).expect("instant"),
        "FREQ=DAILY;UNTIL=20260301T120000Z"
    );
}

#[test]
fn malformed_descriptions_are_refused() {
    let bad = |edit: &dyn Fn(&mut CalendarRecurrenceV1)| {
        let mut rule = simple("weekly");
        edit(&mut rule);
        assert_eq!(build(&rule, &zoned()), Err(CalendarError::InvalidInput));
    };
    bad(&|r| r.freq = "hourly".to_owned());
    bad(&|r| r.freq = String::new());
    bad(&|r| r.interval = 0);
    bad(&|r| r.interval = 1_001);
    bad(&|r| r.count = Some(0));
    bad(&|r| r.count = Some(1_001));
    bad(&|r| {
        r.count = Some(3);
        r.until = Some("2026-03-01".to_owned());
    });
    bad(&|r| r.until = Some("soon".to_owned()));
    bad(&|r| r.by_day = vec!["XX".to_owned()]);
    bad(&|r| r.by_day = vec!["2TU".to_owned()]); // weekly: no number prefix
    bad(&|r| r.by_day = vec!["MO".to_owned(); 8]);
    bad(&|r| r.by_month_day = vec![0]);
    bad(&|r| r.by_month_day = vec![32]);
    bad(&|r| r.by_month = vec![13]);
    bad(&|r| r.by_month = vec![0]);
    bad(&|r| r.by_month = vec![1; 13]);
    // A prefix is fine for a monthly rule, a bad one is not.
    let mut monthly = simple("monthly");
    monthly.by_day = vec!["-1FR".to_owned()];
    assert_eq!(
        build(&monthly, &zoned()).expect("monthly"),
        "FREQ=MONTHLY;BYDAY=-1FR"
    );
    monthly.by_day = vec!["6FR".to_owned()];
    assert_eq!(build(&monthly, &zoned()), Err(CalendarError::InvalidInput));
}
