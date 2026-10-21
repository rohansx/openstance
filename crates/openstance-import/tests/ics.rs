//! ics importer (P3-IMP-01): calendar exports from Google and Apple.

mod common;

use common::*;
use openstance_import::ics::{IcsImporter, MAX_INSTANCES_PER_SERIES};
use openstance_import::*;

/// "Now" for every test: 2026-10-21 12:00 UTC.
fn now() -> i64 {
    utc(2026, 10, 21, 12, 0, 0)
}

fn imp() -> IcsImporter {
    IcsImporter::new(ImportOptions::default()).with_now(now())
}

fn imp_with(opts: ImportOptions) -> IcsImporter {
    IcsImporter::new(opts).with_now(now())
}

fn run_ics_with(i: &IcsImporter, events: &str) -> (ImportStats, MemorySink) {
    let text = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n{events}END:VCALENDAR\r\n"
    );
    let (_dir, p) = temp_file("t.ics", text.as_bytes());
    run(i, &p)
}

fn run_ics(events: &str) -> (ImportStats, MemorySink) {
    run_ics_with(&imp(), events)
}

/// One VEVENT with the given extra lines (CRLF added).
fn ev(lines: &[&str]) -> String {
    let mut s = String::from("BEGIN:VEVENT\r\n");
    for l in lines {
        s.push_str(l);
        s.push_str("\r\n");
    }
    s.push_str("END:VEVENT\r\n");
    s
}

const ORG_OWNER: &str = "ORGANIZER;CN=Owen Ryder:mailto:owner@example.com";
const ATT_OWNER: &str = "ATTENDEE;CN=Owen Ryder;PARTSTAT=ACCEPTED:mailto:owner@example.com";
const ATT_PRIYA: &str = "ATTENDEE;CN=Priya Raman;PARTSTAT=ACCEPTED:mailto:priya@northwind.example";

fn emails(i: &RawInteraction) -> Vec<(Role, String)> {
    i.participants
        .iter()
        .flat_map(|p| {
            p.handles
                .iter()
                .filter(|h| h.kind == HandleKind::Email)
                .map(move |h| (p.role, h.raw.clone()))
        })
        .collect()
}

fn ext_ids(sink: &MemorySink) -> Vec<String> {
    sink.interactions
        .iter()
        .map(|i| i.external_id.clone().unwrap_or_default())
        .collect()
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn detect_strong_on_vcalendar_weak_on_extension() {
    let (_d, p) = temp_file(
        "cal.txt",
        b"\xEF\xBB\xBF\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
    );
    assert_eq!(imp().detect(&p), Detection::Strong);
    let (_d2, p2) = temp_file("cal.ics", b"");
    assert_eq!(imp().detect(&p2), Detection::Weak);
    let (_d3, p3) = temp_file("cards.vcf", b"BEGIN:VCARD\r\nEND:VCARD\r\n");
    assert_eq!(imp().detect(&p3), Detection::No);
    assert_eq!(imp().detect(&fixture("nope.ics")), Detection::No);
    assert_eq!(imp().kind(), "ics");
}

#[test]
fn not_a_calendar_is_wrong_format() {
    let (_d, p) = temp_file("t.ics", b"hello\n");
    let err = imp().import(&p, &mut MemorySink::default()).unwrap_err();
    assert!(
        matches!(err, ImportError::WrongFormat { kind: "ics", .. }),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// Single events
// ---------------------------------------------------------------------------

#[test]
fn simple_event_becomes_one_meeting() {
    let (stats, sink) = run_ics(&ev(&[
        "UID:abc-1@test",
        "SUMMARY:Quarterly\\, planning",
        "DTSTART:20260803T140000Z",
        "DTEND:20260803T150000Z",
        ORG_OWNER,
        ATT_OWNER,
        ATT_PRIYA,
        "DESCRIPTION:SENTINEL-BODY-7f3a",
        "LOCATION:SENTINEL-BODY-7f3a",
    ]));
    assert_eq!(stats.records_read, 1);
    assert_eq!(stats.skipped_total(), 0);
    assert_eq!(sink.interactions.len(), 1);
    let m = &sink.interactions[0];
    assert_eq!(m.kind, InteractionKind::Meeting);
    assert_eq!(m.at, utc(2026, 8, 3, 14, 0, 0));
    assert_eq!(m.subject.as_deref(), Some("Quarterly, planning"));
    assert_eq!(m.external_id.as_deref(), Some("abc-1@test"));
    assert_eq!(m.dir_hint, Some(Direction::Mutual));
    assert!(!m.list_unsub);
    // The organizer is not repeated as an attendee.
    assert_eq!(
        emails(m),
        [
            (Role::Organizer, "owner@example.com".to_string()),
            (Role::Attendee, "priya@northwind.example".to_string()),
        ]
    );
    assert_eq!(m.participants[1].hints.name.as_deref(), Some("Priya Raman"));
    assert_no_sentinel(&sink);
}

#[test]
fn cn_that_is_an_email_is_not_a_name_and_mailto_is_case_insensitive() {
    let (_, sink) = run_ics(&ev(&[
        "UID:u1",
        "DTSTART:20260803T140000Z",
        "ORGANIZER;CN=owner@example.com:MAILTO:owner@example.com",
        "ATTENDEE;CN=\"Raman, Priya\";PARTSTAT=NEEDS-ACTION:mailto:priya@northwind.example",
    ]));
    let m = &sink.interactions[0];
    assert_eq!(m.participants[0].hints.name, None);
    assert_eq!(m.participants[0].handles[0].raw, "owner@example.com");
    assert_eq!(
        m.participants[1].hints.name.as_deref(),
        Some("Raman, Priya")
    );
}

#[test]
fn apple_email_param_with_urn_value() {
    let (_, sink) = run_ics(&ev(&[
        "UID:u1",
        "DTSTART:20260803T140000Z",
        ORG_OWNER,
        "ATTENDEE;CN=Saoirse Delacroix;EMAIL=saoirse@brightwater.example;PARTSTAT=ACCEPTED:urn:uuid:00000000-0000-0000-0000-000000000001",
        "ATTENDEE;CN=No Email;PARTSTAT=ACCEPTED:urn:uuid:00000000-0000-0000-0000-000000000002",
    ]));
    let m = &sink.interactions[0];
    assert_eq!(
        emails(m),
        [
            (Role::Organizer, "owner@example.com".to_string()),
            (Role::Attendee, "saoirse@brightwater.example".to_string()),
        ]
    );
}

#[test]
fn folded_lines_are_unfolded() {
    let (_, sink) = run_ics(
        "BEGIN:VEVENT\r\nUID:fold-1\r\nSUMMARY:A very long\r\n  title\r\nDTSTART:20260803T140000Z\r\nORGANIZER:mailto:owner@exa\r\n mple.com\r\nATTENDEE;CN=Priya\r\n\t Raman:mailto:priya@northwind.example\r\nEND:VEVENT\r\n",
    );
    let m = &sink.interactions[0];
    assert_eq!(m.subject.as_deref(), Some("A very long title"));
    assert_eq!(m.participants[0].handles[0].raw, "owner@example.com");
    assert_eq!(m.participants[1].hints.name.as_deref(), Some("Priya Raman"));
}

#[test]
fn lf_only_line_endings_and_nested_alarms() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:lf-1\nSUMMARY:Sync\nDTSTART:20260803T140000Z\nORGANIZER:mailto:owner@example.com\nATTENDEE:mailto:priya@northwind.example\nBEGIN:VALARM\nACTION:DISPLAY\nDESCRIPTION:Reminder SENTINEL-BODY-7f3a\nTRIGGER:-PT10M\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR\n";
    let (_d, p) = temp_file("lf.ics", text.as_bytes());
    let (stats, sink) = run(&imp(), &p);
    assert_eq!(stats.records_read, 1);
    assert_eq!(sink.interactions.len(), 1);
    assert_eq!(sink.interactions[0].subject.as_deref(), Some("Sync"));
    assert_no_sentinel(&sink);
}

// ---------------------------------------------------------------------------
// Time zones
// ---------------------------------------------------------------------------

#[test]
fn tzid_floating_utc_and_all_day() {
    let opts = ImportOptions {
        tz: chrono_tz::Asia::Kolkata,
        ..Default::default()
    };
    let (_, sink) = run_ics_with(
        &imp_with(opts),
        &[
            ev(&[
                "UID:tz",
                "DTSTART;TZID=America/New_York:20260803T100000",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
            ev(&[
                "UID:tzq",
                "DTSTART;TZID=\"Europe/Berlin\":20260115T100000",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
            ev(&[
                "UID:moz",
                "DTSTART;TZID=/mozilla.org/20050126_1/Europe/Berlin:20260803T160000",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
            ev(&[
                "UID:win",
                "DTSTART;TZID=Pacific Standard Time:20260803T070000",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
            ev(&["UID:float", "DTSTART:20260803T193000", ORG_OWNER, ATT_PRIYA]),
            ev(&["UID:utc", "DTSTART:20260803T140000Z", ORG_OWNER, ATT_PRIYA]),
            ev(&[
                "UID:day",
                "DTSTART;VALUE=DATE:20260915",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
            ev(&[
                "UID:unknown",
                "DTSTART;TZID=Mars/Olympus:20260803T193000",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
        ]
        .concat(),
    );
    let at: Vec<i64> = sink.interactions.iter().map(|i| i.at).collect();
    assert_eq!(
        at,
        [
            utc(2026, 8, 3, 14, 0, 0),   // EDT = UTC-4
            utc(2026, 1, 15, 9, 0, 0),   // CET = UTC+1
            utc(2026, 8, 3, 14, 0, 0),   // CEST = UTC+2
            utc(2026, 8, 3, 14, 0, 0),   // PDT = UTC-7
            utc(2026, 8, 3, 14, 0, 0),   // floating -> ImportOptions.tz (IST = UTC+5:30)
            utc(2026, 8, 3, 14, 0, 0),   // UTC
            utc(2026, 9, 14, 18, 30, 0), // 00:00 IST
            utc(2026, 8, 3, 14, 0, 0),   // unknown TZID -> ImportOptions.tz
        ]
    );
}

#[test]
fn x_wr_timezone_is_the_default_for_floating_times() {
    let text = format!(
        "BEGIN:VCALENDAR\r\nX-WR-TIMEZONE:America/New_York\r\n{}END:VCALENDAR\r\n",
        ev(&["UID:f", "DTSTART:20260803T100000", ORG_OWNER, ATT_PRIYA])
    );
    let (_d, p) = temp_file("t.ics", text.as_bytes());
    let (_, sink) = run(&imp(), &p);
    assert_eq!(sink.interactions[0].at, utc(2026, 8, 3, 14, 0, 0));
}

#[test]
fn bad_or_missing_dtstart_is_skipped() {
    let (stats, sink) = run_ics(
        &[
            ev(&["UID:a", "DTSTART:not-a-date", ORG_OWNER, ATT_PRIYA]),
            ev(&["UID:b", ORG_OWNER, ATT_PRIYA]),
        ]
        .concat(),
    );
    assert_eq!(stats.records_read, 2);
    assert_eq!(stats.skipped.get("bad_date"), Some(&2));
    assert!(sink.interactions.is_empty());
}

// ---------------------------------------------------------------------------
// Skips
// ---------------------------------------------------------------------------

#[test]
fn cancelled_events_are_skipped() {
    let (stats, sink) = run_ics(&ev(&[
        "UID:c",
        "STATUS:CANCELLED",
        "DTSTART:20260803T140000Z",
        ORG_OWNER,
        ATT_PRIYA,
    ]));
    assert!(sink.interactions.is_empty());
    assert_eq!(stats.skipped.get("unsupported"), Some(&1));
}

#[test]
fn declined_attendees_are_dropped() {
    let (_, sink) = run_ics(&ev(&[
        "UID:d",
        "DTSTART:20260803T140000Z",
        ORG_OWNER,
        ATT_PRIYA,
        "ATTENDEE;CN=Ravi Iyer;PARTSTAT=DECLINED:mailto:ravi@contoso.example",
    ]));
    let got: Vec<String> = emails(&sink.interactions[0])
        .into_iter()
        .map(|e| e.1)
        .collect();
    assert_eq!(got, ["owner@example.com", "priya@northwind.example"]);
}

#[test]
fn events_the_owner_declined_are_skipped() {
    // Owner known from [me] identities.
    let opts = ImportOptions {
        me_identities: vec!["email:Owner@Example.com".into()],
        ..Default::default()
    };
    let declined = ev(&[
        "UID:dec",
        "DTSTART:20260803T140000Z",
        "ORGANIZER;CN=Nora:mailto:nora@fabrikam.example",
        "ATTENDEE;PARTSTAT=DECLINED:mailto:owner@example.com",
        ATT_PRIYA,
    ]);
    let (stats, sink) = run_ics_with(&imp_with(opts), &declined);
    assert!(sink.interactions.is_empty());
    assert_eq!(stats.skipped.get("unsupported"), Some(&1));

    // Owner known from a Google export's X-WR-CALNAME.
    let text =
        format!("BEGIN:VCALENDAR\r\nX-WR-CALNAME:owner@example.com\r\n{declined}END:VCALENDAR\r\n");
    let (_d, p) = temp_file("t.ics", text.as_bytes());
    let (stats, sink) = run(&imp(), &p);
    assert!(sink.interactions.is_empty());
    assert_eq!(stats.skipped.get("unsupported"), Some(&1));
    assert_eq!(stats.self_candidates, ["email:owner@example.com"]);
}

#[test]
fn events_without_anyone_but_the_organizer_or_owner_are_skipped() {
    let opts = ImportOptions {
        me_identities: vec!["email:owner@example.com".into()],
        ..Default::default()
    };
    let (stats, sink) = run_ics_with(
        &imp_with(opts),
        &[
            // No attendees at all (focus time).
            ev(&["UID:1", "DTSTART:20260803T140000Z", ORG_OWNER]),
            // Organizer listed again as the only attendee.
            ev(&["UID:2", "DTSTART:20260803T140000Z", ORG_OWNER, ATT_OWNER]),
            // No organizer, the owner is the only attendee.
            ev(&["UID:3", "DTSTART:20260803T140000Z", ATT_OWNER]),
            // Someone else's event with only the owner invited is a meeting.
            ev(&[
                "UID:4",
                "DTSTART:20260803T140000Z",
                "ORGANIZER:mailto:nora@fabrikam.example",
                ATT_OWNER,
            ]),
        ]
        .concat(),
    );
    assert_eq!(stats.records_read, 4);
    assert_eq!(stats.skipped.get("no_participants"), Some(&3));
    assert_eq!(ext_ids(&sink), ["4"]);
}

#[test]
fn rooms_and_resources_are_not_people() {
    let (stats, sink) = run_ics(
        &[
            ev(&[
                "UID:r1",
                "DTSTART:20260803T140000Z",
                ORG_OWNER,
                ATT_PRIYA,
                "ATTENDEE;CUTYPE=ROOM;CN=Room 4:mailto:room4@resource.calendar.example",
                "ATTENDEE;CUTYPE=RESOURCE;CN=Projector:mailto:proj@resource.calendar.example",
            ]),
            // Only a room besides the organizer: nothing to record.
            ev(&[
                "UID:r2",
                "DTSTART:20260803T140000Z",
                ORG_OWNER,
                "ATTENDEE;CUTYPE=ROOM:mailto:room4@resource.calendar.example",
            ]),
        ]
        .concat(),
    );
    assert_eq!(sink.interactions.len(), 1);
    assert_eq!(emails(&sink.interactions[0]).len(), 2);
    assert_eq!(stats.skipped.get("no_participants"), Some(&1));
}

#[test]
fn events_over_max_attendees_are_skipped() {
    let attendees: Vec<String> = (0..6)
        .map(|n| format!("ATTENDEE:mailto:p{n}@northwind.example"))
        .collect();
    let mut lines = vec!["UID:big", "DTSTART:20260803T140000Z", ORG_OWNER];
    lines.extend(attendees.iter().map(String::as_str));
    let big = ev(&lines);
    let opts = ImportOptions {
        ics_max_attendees: 5,
        ..Default::default()
    };
    let (stats, sink) = run_ics_with(&imp_with(opts), &big);
    assert!(sink.interactions.is_empty());
    assert_eq!(stats.skipped.get("unsupported"), Some(&1));
    // Six attendees is fine under the default of 50.
    let (_, sink) = run_ics(&big);
    assert_eq!(sink.interactions.len(), 1);
    assert_eq!(ImportOptions::default().ics_max_attendees, 50);
}

#[test]
fn todos_and_journals_are_ignored() {
    let (stats, sink) = run_ics(
        "BEGIN:VTODO\r\nUID:t\r\nSUMMARY:Buy milk\r\nATTENDEE:mailto:priya@northwind.example\r\nEND:VTODO\r\n",
    );
    assert_eq!(stats.records_read, 0);
    assert!(sink.interactions.is_empty());
}

#[test]
fn unclosed_event_is_malformed() {
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260803T140000Z\r\n";
    let (_d, p) = temp_file("t.ics", text.as_bytes());
    let (stats, sink) = run(&imp(), &p);
    assert!(sink.interactions.is_empty());
    assert_eq!(stats.skipped.get("malformed"), Some(&1));
}

#[test]
fn future_single_events_are_kept() {
    // Only recurrence expansion is limited to the past; a single upcoming
    // meeting is recorded (scoring ignores meetings after now).
    let (_, sink) = run_ics(&ev(&[
        "UID:f",
        "DTSTART:20301015T140000Z",
        ORG_OWNER,
        ATT_PRIYA,
    ]));
    assert_eq!(sink.interactions.len(), 1);
}

// ---------------------------------------------------------------------------
// Recurrence
// ---------------------------------------------------------------------------

#[test]
fn weekly_series_expands_with_exdate_and_overrides() {
    let master = ev(&[
        "UID:weekly@test",
        "SUMMARY:1:1",
        "DTSTART;TZID=America/New_York:20260601T093000",
        "RRULE:FREQ=WEEKLY;COUNT=6",
        "EXDATE;TZID=America/New_York:20260608T093000",
        ORG_OWNER,
        ATT_PRIYA,
    ]);
    // Moved occurrence (listed before the master is fine too).
    let moved = ev(&[
        "UID:weekly@test",
        "SUMMARY:1:1 (moved)",
        "RECURRENCE-ID;TZID=America/New_York:20260615T093000",
        "DTSTART;TZID=America/New_York:20260616T110000",
        ORG_OWNER,
        ATT_PRIYA,
    ]);
    let cancelled = ev(&[
        "UID:weekly@test",
        "STATUS:CANCELLED",
        "RECURRENCE-ID;TZID=America/New_York:20260622T093000",
        "DTSTART;TZID=America/New_York:20260622T093000",
        ORG_OWNER,
        ATT_PRIYA,
    ]);
    let (stats, sink) = run_ics(&[moved, master, cancelled].concat());
    assert_eq!(stats.records_read, 3);
    assert_eq!(
        stats.skipped.get("unsupported"),
        Some(&1),
        "cancelled occurrence"
    );
    // Jun 1, 8 (exdate), 15 (moved to 16th), 22 (cancelled), 29, Jul 6.
    let mut got: Vec<(i64, String, Option<String>)> = sink
        .interactions
        .iter()
        .map(|i| (i.at, i.external_id.clone().unwrap(), i.subject.clone()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            (
                utc(2026, 6, 1, 13, 30, 0),
                "weekly@test/20260601T133000Z".into(),
                Some("1:1".into())
            ),
            (
                utc(2026, 6, 16, 15, 0, 0),
                "weekly@test/20260615T133000Z".into(),
                Some("1:1 (moved)".into())
            ),
            (
                utc(2026, 6, 29, 13, 30, 0),
                "weekly@test/20260629T133000Z".into(),
                Some("1:1".into())
            ),
            (
                utc(2026, 7, 6, 13, 30, 0),
                "weekly@test/20260706T133000Z".into(),
                Some("1:1".into())
            ),
        ]
    );
    for i in &sink.interactions {
        assert_eq!(i.thread_key.as_deref(), Some("weekly@test"));
    }
}

#[test]
fn recurrence_keeps_local_time_across_dst() {
    let (_, sink) = run_ics(&ev(&[
        "UID:dst",
        "DTSTART;TZID=America/New_York:20261026T090000",
        "RRULE:FREQ=WEEKLY;COUNT=3",
        ORG_OWNER,
        ATT_PRIYA,
    ]));
    // Now is 2026-10-21, so all three are in the future.
    assert!(sink.interactions.is_empty());
    let i = IcsImporter::new(ImportOptions::default()).with_now(utc(2027, 1, 1, 0, 0, 0));
    let (_, sink) = run_ics_with(
        &i,
        &ev(&[
            "UID:dst",
            "DTSTART;TZID=America/New_York:20261026T090000",
            "RRULE:FREQ=WEEKLY;COUNT=3",
            ORG_OWNER,
            ATT_PRIYA,
        ]),
    );
    let at: Vec<i64> = sink.interactions.iter().map(|i| i.at).collect();
    assert_eq!(
        at,
        [
            utc(2026, 10, 26, 13, 0, 0), // EDT
            utc(2026, 11, 2, 14, 0, 0),  // EST
            utc(2026, 11, 9, 14, 0, 0),
        ]
    );
}

#[test]
fn only_past_occurrences_capped_to_the_most_recent_104() {
    let (stats, sink) = run_ics(&ev(&[
        "UID:standup",
        "DTSTART:20200106T150000Z",
        "RRULE:FREQ=WEEKLY;BYDAY=MO",
        ORG_OWNER,
        ATT_PRIYA,
    ]));
    assert_eq!(MAX_INSTANCES_PER_SERIES, 104);
    assert_eq!(sink.interactions.len(), 104);
    assert_eq!(stats.skipped_total(), 0);
    let max = sink.interactions.iter().map(|i| i.at).max().unwrap();
    let min = sink.interactions.iter().map(|i| i.at).min().unwrap();
    assert_eq!(max, utc(2026, 10, 19, 15, 0, 0), "last Monday before now");
    assert_eq!(min, max - 103 * 7 * 86_400);
}

#[test]
fn all_future_series_is_skipped() {
    let (stats, sink) = run_ics(&ev(&[
        "UID:later",
        "DTSTART:20300106T150000Z",
        "RRULE:FREQ=MONTHLY;COUNT=5",
        ORG_OWNER,
        ATT_PRIYA,
    ]));
    assert!(sink.interactions.is_empty());
    assert_eq!(stats.skipped.get("unsupported"), Some(&1));
}

#[test]
fn until_and_all_day_series() {
    let (_, sink) = run_ics(
        &[
            ev(&[
                "UID:daily",
                "DTSTART;TZID=Europe/Berlin:20260901T170000",
                "RRULE:FREQ=DAILY;UNTIL=20260905T150000Z",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
            ev(&[
                "UID:monthly-day",
                "DTSTART;VALUE=DATE:20260101",
                "RRULE:FREQ=MONTHLY;COUNT=3",
                "EXDATE;VALUE=DATE:20260201",
                ORG_OWNER,
                ATT_PRIYA,
            ]),
        ]
        .concat(),
    );
    let ids = ext_ids(&sink);
    assert_eq!(
        ids,
        [
            "daily/20260901T150000Z",
            "daily/20260902T150000Z",
            "daily/20260903T150000Z",
            "daily/20260904T150000Z",
            "daily/20260905T150000Z",
            "monthly-day/20260101T000000Z",
            "monthly-day/20260301T000000Z",
        ]
    );
}

#[test]
fn unreadable_rrule_keeps_the_first_occurrence() {
    let (stats, sink) = run_ics(&ev(&[
        "UID:broken",
        "DTSTART:20260601T150000Z",
        "RRULE:FREQ=SOMETIMES",
        ORG_OWNER,
        ATT_PRIYA,
    ]));
    assert_eq!(ext_ids(&sink), ["broken/20260601T150000Z"]);
    assert_eq!(stats.skipped.get("malformed"), Some(&1));
}

#[test]
fn override_without_master_is_still_a_meeting() {
    let (_, sink) = run_ics(&ev(&[
        "UID:orphan",
        "RECURRENCE-ID:20260615T133000Z",
        "DTSTART:20260616T150000Z",
        ORG_OWNER,
        ATT_PRIYA,
    ]));
    assert_eq!(ext_ids(&sink), ["orphan/20260615T133000Z"]);
}

// ---------------------------------------------------------------------------
// Committed fixtures
// ---------------------------------------------------------------------------

#[test]
fn google_fixture() {
    let (stats, sink) = run(&imp(), &fixture("ics/google.ics"));
    assert_eq!(stats.records_read, 13);
    assert_eq!(stats.skipped.get("unsupported"), Some(&5));
    assert_eq!(stats.skipped.get("no_participants"), Some(&2));
    assert_eq!(stats.skipped_total(), 7);
    assert_eq!(sink.interactions.len(), 117);
    assert_eq!(stats.self_candidates, ["email:owner@example.com"]);
    assert!(sink
        .interactions
        .iter()
        .all(|i| i.kind == InteractionKind::Meeting));
    // Rooms, declined attendees and the all-hands crowd never appear.
    let all = handles(&sink, HandleKind::Email);
    assert!(!all.iter().any(|h| h.contains("resource.calendar")));
    assert!(!all.iter().any(|h| h.starts_with("allhands-")));
    assert!(!all.contains(&"ravi@contoso.example".to_string()));
    assert_no_sentinel(&sink);
}

#[test]
fn apple_fixture() {
    let (stats, sink) = run(&imp(), &fixture("ics/apple.ics"));
    assert_eq!(stats.records_read, 4);
    assert_eq!(stats.skipped_total(), 0);
    assert_eq!(sink.interactions.len(), 8);
    let all = handles(&sink, HandleKind::Email);
    assert!(all.contains(&"saoirse@brightwater.example".to_string()));
    assert!(all.contains(&"quinn.harlow@lumen.example".to_string()));
    assert_no_sentinel(&sink);
}

#[test]
fn the_same_meeting_in_both_exports_has_the_same_external_id() {
    let (_, g) = run(&imp(), &fixture("ics/google.ics"));
    let (_, a) = run(&imp(), &fixture("ics/apple.ics"));
    let shared: Vec<_> = g
        .interactions
        .iter()
        .filter(|gi| {
            a.interactions
                .iter()
                .any(|ai| ai.external_id == gi.external_id && ai.at == gi.at)
        })
        .collect();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].subject.as_deref(), Some("Northwind sync"));
}
