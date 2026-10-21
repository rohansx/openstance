//! Phase 3 importer fixtures (P3-IMP-01/02): calendar exports under
//! `fixtures/ics/` and an X archive under `fixtures/x/` (the unzipped
//! folder plus the same files as `twitter-archive.zip`).
//!
//! They are not part of the Phase 1 load set, so the Phase 1 contract
//! totals in `EXPECTED.md` do not change; the section written by
//! [`expected_md_section`] states what these files must produce.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::{BODY_SENTINEL, OWNER_EMAIL, OWNER_NAME};

/// The owner's X account id in the archive.
pub const OWNER_X_ID: &str = "1000000001";
/// The owner's X username in the archive.
pub const OWNER_X_HANDLE: &str = "owenryder_x";

/// Expected importer output for the Phase 3 fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase3Expected {
    /// VEVENTs in `ics/google.ics`.
    pub google_events: u64,
    /// Meetings from `ics/google.ics`.
    pub google_meetings: u64,
    /// Meetings from `ics/apple.ics`.
    pub apple_meetings: u64,
    /// The meetings both exports share (same UID and instant).
    pub shared_meetings: u64,
    /// Follow interactions from the archive.
    pub follows: u64,
    /// DM interactions from the archive.
    pub dms: u64,
    /// Distinct X account ids in the archive (owner included).
    pub x_ids: u64,
}

/// The numbers the importer tests and `EXPECTED.md` state.
pub fn expected() -> Phase3Expected {
    Phase3Expected {
        google_events: 13,
        google_meetings: 117,
        apple_meetings: 8,
        shared_meetings: 1,
        follows: FOLLOWING.len() as u64 + FOLLOWERS.len() as u64,
        dms: 7,
        x_ids: 13,
    }
}

const CRLF: &str = "\r\n";

fn cal(header: &[&str], events: &[String]) -> String {
    let mut out = String::new();
    for l in ["BEGIN:VCALENDAR", "VERSION:2.0"] {
        out.push_str(l);
        out.push_str(CRLF);
    }
    for l in header {
        out.push_str(l);
        out.push_str(CRLF);
    }
    for e in events {
        out.push_str(e);
    }
    out.push_str("END:VCALENDAR");
    out.push_str(CRLF);
    out
}

fn ev(lines: &[&str]) -> String {
    let mut s = String::from("BEGIN:VEVENT\r\n");
    for l in lines {
        s.push_str(l);
        s.push_str(CRLF);
    }
    s.push_str("END:VEVENT\r\n");
    s
}

fn owner_org() -> String {
    format!("ORGANIZER;CN={OWNER_NAME}:mailto:{OWNER_EMAIL}")
}

fn owner_att(partstat: &str) -> String {
    format!(
        "ATTENDEE;CN={OWNER_NAME};PARTSTAT={partstat};ROLE=REQ-PARTICIPANT:mailto:{OWNER_EMAIL}"
    )
}

const NY_TZ: &str = "BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\nBEGIN:DAYLIGHT\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nTZNAME:EDT\r\nDTSTART:19700308T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\nEND:DAYLIGHT\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nTZNAME:EST\r\nDTSTART:19701101T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";

/// A Google Calendar export of the owner's primary calendar.
pub fn google_ics() -> String {
    let org = owner_org();
    let me = owner_att("ACCEPTED");
    let priya = "ATTENDEE;CN=Priya Raman;PARTSTAT=ACCEPTED;ROLE=REQ-PARTICIPANT:mailto:priya@northwind.example";
    let marcus = "ATTENDEE;CN=Marcus Lee;PARTSTAT=ACCEPTED:mailto:marcus@contoso.example";
    let desc = format!("DESCRIPTION:Agenda {BODY_SENTINEL}\\nJoin: https://meet.example/abc");
    let loc = format!("LOCATION:{BODY_SENTINEL} Room 4");
    let mut events = vec![NY_TZ.to_string()];
    // E1: one-off sync with a room; also in the Apple export (same UID).
    events.push(ev(&[
        "DTSTART;TZID=America/New_York:20260803T100000",
        "DTEND;TZID=America/New_York:20260803T110000",
        "UID:northwind-sync-0803@openstance.test",
        &org,
        &me,
        priya,
        marcus,
        "ATTENDEE;CUTYPE=RESOURCE;CN=NW Room 4;PARTSTAT=ACCEPTED:mailto:nw-room-4@resource.calendar.example",
        "SUMMARY:Northwind sync",
        &desc,
        &loc,
        "STATUS:CONFIRMED",
    ]));
    // E2: weekly 1:1 with Priya, 12 weeks, one EXDATE, one moved and one
    // cancelled occurrence -> 10 meetings.
    let priya_org = "ORGANIZER;CN=Priya Raman:mailto:priya@northwind.example";
    events.push(ev(&[
        "DTSTART;TZID=America/New_York:20260601T093000",
        "DTEND;TZID=America/New_York:20260601T100000",
        "RRULE:FREQ=WEEKLY;COUNT=12;BYDAY=MO",
        "EXDATE;TZID=America/New_York:20260615T093000",
        "UID:priya-1on1@openstance.test",
        priya_org,
        priya,
        &me,
        "SUMMARY:Priya / Owen 1:1",
        &desc,
    ]));
    events.push(ev(&[
        "DTSTART;TZID=America/New_York:20260623T110000",
        "DTEND;TZID=America/New_York:20260623T113000",
        "RECURRENCE-ID;TZID=America/New_York:20260622T093000",
        "UID:priya-1on1@openstance.test",
        priya_org,
        priya,
        &me,
        "SUMMARY:Priya / Owen 1:1 (moved)",
    ]));
    events.push(ev(&[
        "DTSTART;TZID=America/New_York:20260629T093000",
        "RECURRENCE-ID;TZID=America/New_York:20260629T093000",
        "UID:priya-1on1@openstance.test",
        priya_org,
        priya,
        &me,
        "SUMMARY:Priya / Owen 1:1",
        "STATUS:CANCELLED",
    ]));
    // E3: a standup that has run every Monday since 2023 -> capped at 104.
    events.push(ev(&[
        "DTSTART;TZID=America/New_York:20230102T090000",
        "DTEND;TZID=America/New_York:20230102T091500",
        "RRULE:FREQ=WEEKLY;BYDAY=MO",
        "UID:contoso-standup@openstance.test",
        "ORGANIZER;CN=Marcus Lee:mailto:marcus@contoso.example",
        marcus,
        &me,
        "ATTENDEE;CN=Kofi Mensah;PARTSTAT=TENTATIVE:mailto:kofi@tailspin.example",
        "SUMMARY:Contoso standup",
        "BEGIN:VALARM",
        "ACTION:DISPLAY",
        &format!("DESCRIPTION:{BODY_SENTINEL}"),
        "TRIGGER:-PT10M",
        "END:VALARM",
    ]));
    // E5: a series that has not started yet -> skipped.
    events.push(ev(&[
        "DTSTART:20300107T150000Z",
        "RRULE:FREQ=MONTHLY;COUNT=6",
        "UID:future-board@openstance.test",
        &org,
        priya,
        "SUMMARY:Board prep",
    ]));
    // E6: the owner declined -> skipped.
    events.push(ev(&[
        "DTSTART:20260812T170000Z",
        "UID:nora-lunch@openstance.test",
        "ORGANIZER;CN=Nora Lindqvist:mailto:nora@fabrikam.example",
        "ATTENDEE;CN=Nora Lindqvist;PARTSTAT=ACCEPTED:mailto:nora@fabrikam.example",
        &owner_att("DECLINED"),
        "SUMMARY:Lunch?",
    ]));
    // E7: cancelled -> skipped.
    events.push(ev(&[
        "DTSTART:20260820T150000Z",
        "UID:offsite-planning@openstance.test",
        &org,
        priya,
        marcus,
        "SUMMARY:Offsite planning",
        "STATUS:CANCELLED",
    ]));
    // E8: all-hands with 60 attendees -> skipped (ics_max_attendees = 50).
    let crowd: Vec<String> = (1..=60)
        .map(|n| format!("ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:allhands-{n:02}@northwind.example"))
        .collect();
    let mut allhands: Vec<&str> = vec![
        "DTSTART:20260904T160000Z",
        "UID:northwind-allhands@openstance.test",
        "ORGANIZER;CN=Northwind People Team:mailto:people@northwind.example",
        "SUMMARY:Northwind all hands",
    ];
    allhands.extend(crowd.iter().map(String::as_str));
    events.push(ev(&allhands));
    // E9: focus time, no attendees -> skipped.
    events.push(ev(&[
        "DTSTART:20260806T130000Z",
        "UID:focus@openstance.test",
        &org,
        "SUMMARY:Focus time",
    ]));
    // E10: only the owner -> skipped.
    events.push(ev(&[
        "DTSTART:20260807T130000Z",
        "UID:dentist@openstance.test",
        &org,
        &me,
        "SUMMARY:Dentist",
    ]));
    // E11: coffee organized by Elena (UTC); Ravi declined and is dropped.
    events.push(ev(&[
        "DTSTART:20260910T150000Z",
        "DTEND:20260910T153000Z",
        "UID:elena-coffee@openstance.test",
        "ORGANIZER;CN=\"Voss, Elena\":mailto:elena@tailspin.example",
        "ATTENDEE;CN=\"Voss, Elena\";PARTSTAT=ACCEPTED:mailto:elena@tailspin.example",
        &me,
        "ATTENDEE;CN=Ravi Iyer;PARTSTAT=DECLINED:mailto:ravi@contoso.example",
        "SUMMARY:Coffee \\, catch-up",
        &desc,
    ]));
    // E12: all-day design review with Lena.
    events.push(ev(&[
        "DTSTART;VALUE=DATE:20260915",
        "DTEND;VALUE=DATE:20260916",
        "UID:design-review-day@openstance.test",
        &org,
        "ATTENDEE;CN=Lena Park;PARTSTAT=ACCEPTED:mailto:lena.park@mail.test",
        "SUMMARY:Design review day",
    ]));
    cal(
        &[
            "PRODID:-//Google Inc//Google Calendar 70.9054//EN",
            "CALSCALE:GREGORIAN",
            "METHOD:PUBLISH",
            &format!("X-WR-CALNAME:{OWNER_EMAIL}"),
            "X-WR-TIMEZONE:America/New_York",
        ],
        &events,
    )
}

/// An Apple Calendar export of a calendar named "Work".
pub fn apple_ics() -> String {
    let org = owner_org();
    let me = owner_att("ACCEPTED");
    let events = vec![
        // A1: floating time, a new person.
        ev(&[
            "UID:3F1D2A6E-0001-4C1B-9E55-OPENSTANCE",
            "DTSTART:20260805T120000",
            "DTEND:20260805T130000",
            "SUMMARY:Intro: Quinn x Priya",
            &org,
            &me,
            "ATTENDEE;CN=Quinn Harlow;CUTYPE=INDIVIDUAL;PARTSTAT=ACCEPTED;EMAIL=quinn.harlow@lumen.example:mailto:quinn.harlow@lumen.example",
            "ATTENDEE;CN=Priya Raman;PARTSTAT=ACCEPTED:mailto:priya@northwind.example",
            &format!("X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-TITLE={BODY_SENTINEL}:geo:0,0"),
        ]),
        // A2: the Google E1 meeting again, written in Berlin time.
        ev(&[
            "UID:northwind-sync-0803@openstance.test",
            "DTSTART;TZID=Europe/Berlin:20260803T160000",
            "DTEND;TZID=Europe/Berlin:20260803T170000",
            "SUMMARY:Northwind sync",
            &org,
            &me,
            "ATTENDEE;CN=Priya Raman;PARTSTAT=ACCEPTED:mailto:priya@northwind.example",
            "ATTENDEE;CN=Marcus Lee;PARTSTAT=ACCEPTED:mailto:marcus@contoso.example",
        ]),
        // A3: Apple's EMAIL parameter with a urn:uuid value, folded line.
        ev(&[
            "UID:3F1D2A6E-0003-4C1B-9E55-OPENSTANCE",
            "DTSTART;TZID=Europe/Berlin:20260818T150000",
            "SUMMARY:Tea",
            &org,
            &me,
            "ATTENDEE;CN=Saoirse Delacroix;CUTYPE=INDIVIDUAL;EMAIL=saoirse@brightwat\r\n er.example;PARTSTAT=ACCEPTED:urn:uuid:5c1e0000-0000-4000-8000-000000000003",
            &format!("DESCRIPTION:{BODY_SENTINEL}"),
        ]),
        // A4: a daily series ending with UNTIL -> 5 meetings.
        ev(&[
            "UID:3F1D2A6E-0004-4C1B-9E55-OPENSTANCE",
            "DTSTART;TZID=Europe/Berlin:20260901T170000",
            "RRULE:FREQ=DAILY;UNTIL=20260905T150000Z",
            "SUMMARY:Lumen onboarding",
            &org,
            &me,
            "ATTENDEE;CN=Quinn Harlow;PARTSTAT=ACCEPTED:mailto:quinn.harlow@lumen.example",
        ]),
    ];
    cal(
        &[
            "PRODID:-//Apple Inc.//macOS 15.0//EN",
            "CALSCALE:GREGORIAN",
            "X-WR-CALNAME:Work",
        ],
        &events,
    )
}

const FOLLOWING: [&str; 8] = [
    "2000000001",
    "2000000002",
    "2000000003",
    "2000000004",
    "2000000005",
    "2000000006",
    "2000000007",
    "2000000008",
];
const FOLLOWERS: [&str; 6] = [
    "2000000006",
    "2000000007",
    "2000000008",
    "2000000009",
    "2000000010",
    "2000000011",
];

fn follow_js(name: &str, ids: &[&str]) -> String {
    let items: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                "  {{\n    \"{name}\" : {{\n      \"accountId\" : \"{id}\",\n      \"userLink\" : \"https://twitter.com/intent/user?user_id={id}\"\n    }}\n  }}"
            )
        })
        .collect();
    format!("window.YTD.{name}.part0 = [\n{}\n]", items.join(",\n"))
}

fn message(sender: &str, recipient: Option<&str>, id: &str, at: &str) -> String {
    let r = recipient
        .map(|r| format!("\"recipientId\" : \"{r}\",\n          "))
        .unwrap_or_default();
    format!(
        "{{\n        \"messageCreate\" : {{\n          {r}\"reactions\" : [ ],\n          \"urls\" : [ ],\n          \"text\" : \"{BODY_SENTINEL}\",\n          \"mediaUrls\" : [ ],\n          \"senderId\" : \"{sender}\",\n          \"id\" : \"{id}\",\n          \"createdAt\" : \"{at}\"\n        }}\n      }}"
    )
}

fn conversation(id: &str, items: &[String]) -> String {
    format!(
        "  {{\n    \"dmConversation\" : {{\n      \"conversationId\" : \"{id}\",\n      \"messages\" : [ {} ]\n    }}\n  }}",
        items.join(", ")
    )
}

/// `fixtures/x/archive/data/*.js`: relative path -> contents.
pub fn x_files() -> BTreeMap<&'static str, String> {
    let o = OWNER_X_ID;
    let mut f = BTreeMap::new();
    f.insert(
        "x/archive/data/manifest.js",
        format!(
            "window.__THAR_CONFIG = {{\n  \"userInfo\" : {{\n    \"accountId\" : \"{o}\",\n    \"userName\" : \"{OWNER_X_HANDLE}\",\n    \"displayName\" : \"{OWNER_NAME}\"\n  }},\n  \"archiveInfo\" : {{\n    \"sizeBytes\" : \"4096\",\n    \"generationDate\" : \"2026-09-30T12:00:00.000Z\",\n    \"isPartialArchive\" : false,\n    \"maxPartSizeBytes\" : \"53687091200\"\n  }}\n}}"
        ),
    );
    f.insert(
        "x/archive/data/account.js",
        format!(
            "window.YTD.account.part0 = [\n  {{\n    \"account\" : {{\n      \"email\" : \"{OWNER_EMAIL}\",\n      \"createdVia\" : \"web\",\n      \"username\" : \"{OWNER_X_HANDLE}\",\n      \"accountId\" : \"{o}\",\n      \"createdAt\" : \"2014-05-01T08:00:00.000Z\",\n      \"accountDisplayName\" : \"{OWNER_NAME}\"\n    }}\n  }}\n]"
        ),
    );
    f.insert(
        "x/archive/data/following.js",
        follow_js("following", &FOLLOWING),
    );
    f.insert(
        "x/archive/data/follower.js",
        follow_js("follower", &FOLLOWERS),
    );
    let reaction = format!(
        "{{\n        \"reactionCreate\" : {{\n          \"senderId\" : \"{o}\",\n          \"reactionKey\" : \"like\",\n          \"eventId\" : \"1820000000000000100\",\n          \"createdAt\" : \"2026-08-01T10:06:00.000Z\"\n        }}\n      }}"
    );
    let a = "2000000001";
    let b = "2000000009";
    let dms = [
        conversation(
            &format!("{o}-{a}"),
            &[
                message(
                    a,
                    Some(o),
                    "1820000000000000001",
                    "2026-08-01T10:00:00.000Z",
                ),
                message(
                    o,
                    Some(a),
                    "1820000000000000002",
                    "2026-08-01T10:05:00.000Z",
                ),
                reaction,
                message(
                    a,
                    Some(o),
                    "1820000000000000003",
                    "2026-08-02T09:00:00.000Z",
                ),
            ],
        ),
        conversation(
            &format!("{o}-{b}"),
            &[
                message(
                    o,
                    Some(b),
                    "1820000000000000004",
                    "2026-09-01T12:00:00.000Z",
                ),
                message(
                    b,
                    Some(o),
                    "1820000000000000005",
                    "2026-09-02T12:00:00.000Z",
                ),
            ],
        ),
    ];
    f.insert(
        "x/archive/data/direct-messages.js",
        format!(
            "window.YTD.direct_messages.part0 = [\n{}\n]",
            dms.join(",\n")
        ),
    );
    let c = "2000000012";
    let group = conversation(
        "1600000000000000001",
        &[
            message(c, None, "1820000000000000006", "2026-09-10T18:00:00.000Z"),
            message(o, None, "1820000000000000007", "2026-09-10T18:05:00.000Z"),
            format!(
                "{{\n        \"conversationNameUpdate\" : {{\n          \"initiatingUserId\" : \"{c}\",\n          \"name\" : \"{BODY_SENTINEL}\",\n          \"createdAt\" : \"2026-09-10T17:59:00.000Z\"\n        }}\n      }}"
            ),
            format!(
                "{{\n        \"joinConversation\" : {{\n          \"initiatingUserId\" : \"{c}\",\n          \"participantsSnapshot\" : [ \"{o}\", \"{a}\", \"{c}\" ],\n          \"createdAt\" : \"2026-09-10T17:58:00.000Z\"\n        }}\n      }}"
            ),
        ],
    );
    f.insert(
        "x/archive/data/direct-messages-group.js",
        format!("window.YTD.direct_messages_group.part0 = [\n{group}\n]"),
    );
    // Not read by the importer (tweets carry free text).
    f.insert(
        "x/archive/data/tweets.js",
        format!(
            "window.YTD.tweets.part0 = [\n  {{\n    \"tweet\" : {{\n      \"id\" : \"1820000000000000900\",\n      \"full_text\" : \"{BODY_SENTINEL}\",\n      \"created_at\" : \"Tue Sep 01 12:00:00 +0000 2026\"\n    }}\n  }}\n]"
        ),
    );
    f
}

/// Every Phase 3 text fixture: relative path -> contents.
pub fn files() -> BTreeMap<&'static str, String> {
    let mut f = BTreeMap::new();
    f.insert("ics/google.ics", google_ics());
    f.insert("ics/apple.ics", apple_ics());
    f.extend(x_files());
    f
}

/// Binary fixtures: `x/twitter-archive.zip` (the archive folder, stored).
pub fn binary_files() -> BTreeMap<&'static str, Vec<u8>> {
    let entries: Vec<(String, Vec<u8>)> = x_files()
        .into_iter()
        .map(|(p, t)| {
            (
                p.strip_prefix("x/archive/").unwrap_or(p).to_string(),
                t.into_bytes(),
            )
        })
        .collect();
    let mut f = BTreeMap::new();
    f.insert("x/twitter-archive.zip", stored_zip(&entries));
    f
}

// ---------------------------------------------------------------------------
// A minimal deterministic zip writer (stored entries, fixed timestamps)
// ---------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A zip of `(name, bytes)` with no compression and a fixed DOS time
/// (2026-09-30 12:00), so the bytes are reproducible.
pub fn stored_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    const TIME: u16 = 12 << 11;
    const DATE: u16 = ((2026 - 1980) << 9) | (9 << 5) | 30;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let n = name.as_bytes();
        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes()); // version needed
        local.extend_from_slice(&0u16.to_le_bytes()); // flags
        local.extend_from_slice(&0u16.to_le_bytes()); // stored
        local.extend_from_slice(&TIME.to_le_bytes());
        local.extend_from_slice(&DATE.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&(n.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(n);
        out.extend_from_slice(&local);
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&20u16.to_le_bytes()); // needed
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&TIME.to_le_bytes());
        central.extend_from_slice(&DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(n.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(n);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The `EXPECTED.md` section for the Phase 3 importer fixtures.
pub fn expected_md_section() -> String {
    let e = expected();
    let mut out = String::new();
    out.push_str("\n## Phase 3 importer fixtures (P3-IMP-01/02)\n\n");
    out.push_str("Not part of the Phase 1 load set above, so the totals above do not change. Importer output with \"now\" after 2026-10-01:\n\n");
    out.push_str("| File | Expected | Notes |\n|---|---|---|\n");
    let _ = writeln!(
        out,
        "| `ics/google.ics` | {} meetings from {} VEVENTs | 1 one-off sync (room dropped) + 10 of a 12-week 1:1 (one `EXDATE`, one moved, one cancelled occurrence) + 104 of a weekly standup running since 2023 (cap per series) + Elena's coffee (declined Ravi dropped) + an all-day review; skipped: `unsupported` 5 (cancelled event, cancelled occurrence, owner declined, 60-person all-hands, not-yet-started series), `no_participants` 2 (focus time, owner-only) |",
        e.google_meetings, e.google_events
    );
    let _ = writeln!(
        out,
        "| `ics/apple.ics` | {} meetings from 4 VEVENTs | floating time, `TZID=Europe/Berlin`, `EMAIL=` parameter with a `urn:uuid` value on a folded line, a daily series with `UNTIL` (5); new people Quinn Harlow and Saoirse Delacroix |",
        e.apple_meetings
    );
    let _ = writeln!(
        out,
        "| both `.ics` files | {} shared meeting | \"Northwind sync\" has the same UID and instant in both exports and is stored once (global meeting dedupe) |",
        e.shared_meetings
    );
    let _ = writeln!(
        out,
        "| `x/twitter-archive.zip` (= `x/archive/`) | {} follow + {} dm interactions | {} following + {} followers (3 overlap), 5 one-to-one messages in 2 conversations (a reaction is not a message), 2 group messages; {} distinct account ids; self candidates `x_user_id:{OWNER_X_ID}`, `x_handle:{OWNER_X_HANDLE}` |",
        e.follows,
        e.dms,
        FOLLOWING.len(),
        FOLLOWERS.len(),
        e.x_ids
    );
    out.push_str("\nLoaded on top of the Phase 1 set (with `x_user_id:1000000001` added to `[me]`): 2 new named people from the calendars, 3 new X-only people from DMs (ids only, no names), and the 9 follow-only ids stay dormant (no person). Flagged merges and suggestions are unchanged.\n");
    let _ = writeln!(out, "No record carries `{BODY_SENTINEL}` (descriptions, locations, alarms, DM text, conversation names and tweets are never read).");
    out
}
