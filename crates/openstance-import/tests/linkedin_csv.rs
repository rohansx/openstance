//! linkedin_csv importer (P1-IMP-03): Connections.csv, messages.csv,
//! Invitations.csv, plus the CSV edge cases ported from the v0.1 prototype.

mod common;

use common::*;
use openstance_import::linkedin_csv::LinkedinCsvImporter;
use openstance_import::*;

fn imp() -> LinkedinCsvImporter {
    LinkedinCsvImporter::new(ImportOptions::default())
}

fn run_csv(name: &str, content: &str) -> (ImportStats, MemorySink) {
    let (_dir, p) = temp_file(name, content.as_bytes());
    run(&imp(), &p)
}

const CONN_HEADER: &str = "First Name,Last Name,URL,Email Address,Company,Position,Connected On";

// ---------------------------------------------------------------------------
// Connections.csv fixture
// ---------------------------------------------------------------------------

#[test]
fn connections_fixture_one_interaction_per_row() {
    let (stats, sink) = run(&imp(), &fixture("linkedin/Connections.csv"));
    assert_eq!(stats.records_read, 12);
    assert_eq!(stats.skipped_total(), 0);
    assert_eq!(sink.interactions.len(), 12);
    assert!(sink.identities.is_empty());
    for i in &sink.interactions {
        assert_eq!(i.kind, InteractionKind::Connection);
        assert_eq!(i.dir_hint, Some(Direction::Mutual));
        assert!(!i.list_unsub);
        assert_eq!(i.participants.len(), 1);
        assert_eq!(i.participants[0].role, Role::Subject);
        assert_eq!(i.participants[0].handles[0].kind, HandleKind::LinkedinUrl);
    }
    assert_no_sentinel(&sink);
}

#[test]
fn connections_fixture_first_row_mapping() {
    let (_, sink) = run(&imp(), &fixture("linkedin/Connections.csv"));
    let priya = &sink.interactions[0];
    assert_eq!(priya.at, utc(2024, 3, 12, 0, 0, 0));
    assert_eq!(priya.external_id.as_deref(), Some("conn:priya-raman-nw"));
    let p = &priya.participants[0];
    assert_eq!(
        p.handles,
        vec![
            Handle::new(
                HandleKind::LinkedinUrl,
                "https://www.linkedin.com/in/priya-raman-nw"
            ),
            Handle::new(HandleKind::Email, "priya@northwind.example"),
        ]
    );
    assert_eq!(p.hints.name.as_deref(), Some("Priya Raman"));
    assert_eq!(p.hints.company.as_deref(), Some("Northwind"));
    assert_eq!(p.hints.title.as_deref(), Some("VP Engineering"));
    assert_eq!(p.hints.location, None);
}

#[test]
fn connections_fixture_empty_email_and_quoted_comma() {
    let (_, sink) = run(&imp(), &fixture("linkedin/Connections.csv"));
    let sam = &sink.interactions[2].participants[0];
    assert_eq!(sam.handles.len(), 1, "empty Email Address adds no handle");
    let sofia = sink
        .interactions
        .iter()
        .find(|i| i.external_id.as_deref() == Some("conn:sofia-marchetti"))
        .unwrap();
    assert_eq!(
        sofia.participants[0].hints.title.as_deref(),
        Some("Partner, Seed Fund")
    );
    let emails = handles(&sink, HandleKind::Email);
    assert_eq!(
        emails,
        ["priya@northwind.example", "marcus@contoso.example"]
    );
}

// ---------------------------------------------------------------------------
// messages.csv fixture
// ---------------------------------------------------------------------------

#[test]
fn messages_fixture_dm_rows() {
    let (stats, sink) = run(&imp(), &fixture("linkedin/messages.csv"));
    assert_eq!(stats.records_read, 5);
    assert_eq!(sink.interactions.len(), 5);
    assert_no_sentinel(&sink);
    let first = &sink.interactions[0];
    assert_eq!(first.kind, InteractionKind::Dm);
    assert_eq!(first.at, utc(2026, 8, 3, 10, 0, 0));
    assert_eq!(first.thread_key.as_deref(), Some("2-ZmFrZS1wcml5YS1jb252"));
    assert_eq!(
        first.external_id.as_deref(),
        Some("msg:2-ZmFrZS1wcml5YS1jb252:2026-08-03 10:00:00 UTC:owen-ryder-example")
    );
    assert_eq!(first.subject, None);
    assert_eq!(first.dir_hint, Some(Direction::Out), "FOLDER=SENT");
    assert_eq!(
        sink.interactions[1].dir_hint,
        Some(Direction::In),
        "FOLDER=INBOX"
    );
    let roles: Vec<_> = first.participants.iter().map(|p| p.role).collect();
    assert_eq!(roles, [Role::Sender, Role::Recipient]);
    assert_eq!(
        first.participants[0].hints.name.as_deref(),
        Some("Owen Ryder")
    );
    assert_eq!(
        first.participants[1].hints.name.as_deref(),
        Some("Priya Raman")
    );
    assert_eq!(
        first.participants[1].handles,
        vec![Handle::new(
            HandleKind::LinkedinUrl,
            "https://www.linkedin.com/in/priya-raman-nw"
        )]
    );
}

#[test]
fn messages_fixture_subject_falls_back_to_title_and_self_candidate() {
    let (stats, sink) = run(&imp(), &fixture("linkedin/messages.csv"));
    let hugo = &sink.interactions[2];
    assert_eq!(
        hugo.subject.as_deref(),
        Some("Speaking slot at Devtools Days")
    );
    assert_eq!(
        stats.self_candidates,
        vec!["linkedin_url:owen-ryder-example".to_string()]
    );
}

#[test]
fn messages_subject_falls_back_to_conversation_title() {
    let (_, sink) = run_csv(
        "messages.csv",
        "CONVERSATION ID,CONVERSATION TITLE,FROM,SENDER PROFILE URL,TO,RECIPIENT PROFILE URLS,DATE,SUBJECT,CONTENT,FOLDER\n\
         c1,Group chat,Ann Example,https://www.linkedin.com/in/ann-example,\"Bo Example, Cy Example\",\"https://www.linkedin.com/in/bo-example https://www.linkedin.com/in/cy-example\",2026-01-02 03:04:05 UTC,,hi,INBOX\n",
    );
    let i = &sink.interactions[0];
    assert_eq!(i.subject.as_deref(), Some("Group chat"));
    assert_eq!(i.participants.len(), 3);
    assert_eq!(i.participants[2].hints.name.as_deref(), Some("Cy Example"));
    assert_eq!(
        i.participants[2].handles[0].raw,
        "https://www.linkedin.com/in/cy-example"
    );
}

#[test]
fn messages_spam_and_anonymous_members_are_skipped() {
    let (stats, sink) = run_csv(
        "messages.csv",
        "CONVERSATION ID,CONVERSATION TITLE,FROM,SENDER PROFILE URL,TO,RECIPIENT PROFILE URLS,DATE,SUBJECT,CONTENT,FOLDER\n\
         c1,,Spammer,https://www.linkedin.com/in/spammer,Me,https://www.linkedin.com/in/me,2026-01-02 03:04:05 UTC,,buy,SPAM\n\
         c2,,LinkedIn Member,,Me,https://www.linkedin.com/in/me,2026-01-02 03:04:05 UTC,,InMail,INBOX\n\
         c3,,Ann Example,https://www.linkedin.com/in/ann-example,Me,https://www.linkedin.com/in/me,not a date,,x,INBOX\n",
    );
    assert_eq!(stats.records_read, 3);
    assert_eq!(sink.interactions.len(), 0);
    assert_eq!(stats.skipped.get("unsupported"), Some(&2));
    assert_eq!(stats.skipped.get("bad_date"), Some(&1));
    assert_eq!(
        sink.skips[0],
        (SkipReason::Unsupported, "row 1".to_string())
    );
}

#[test]
fn opaque_member_urls_join_sibling_connections_for_unique_names() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Connections.csv"),
        format!(
            "{CONN_HEADER}\n\
             Ann,Example,https://www.linkedin.com/in/ann-example,,Acme,PM,01 Jan 2025\n\
             Dup,Name,https://www.linkedin.com/in/dup-name-1,,Acme,PM,01 Jan 2025\n\
             Dup,Name,https://www.linkedin.com/in/dup-name-2,,Acme,PM,01 Jan 2025\n"
        ),
    )
    .unwrap();
    let msgs = dir.path().join("messages.csv");
    std::fs::write(
        &msgs,
        "CONVERSATION ID,CONVERSATION TITLE,FROM,SENDER PROFILE URL,TO,RECIPIENT PROFILE URLS,DATE,SUBJECT,CONTENT,FOLDER\n\
         c1,,Ann Example,https://www.linkedin.com/in/ACoAABxyz123,Me,https://www.linkedin.com/in/me,2026-01-02 03:04:05 UTC,,x,INBOX\n\
         c2,,Dup Name,https://www.linkedin.com/in/ACoAABdup999,Me,https://www.linkedin.com/in/me,2026-01-02 03:04:05 UTC,,x,INBOX\n",
    )
    .unwrap();
    let (_, sink) = run(&imp(), &msgs);
    let ann = &sink.interactions[0].participants[0];
    assert_eq!(
        ann.handles,
        vec![
            Handle::new(
                HandleKind::LinkedinUrl,
                "https://www.linkedin.com/in/ACoAABxyz123"
            ),
            Handle::new(
                HandleKind::LinkedinUrl,
                "https://www.linkedin.com/in/ann-example"
            ),
        ]
    );
    let dup = &sink.interactions[1].participants[0];
    assert_eq!(dup.handles.len(), 1, "ambiguous names are not joined");
}

// ---------------------------------------------------------------------------
// Invitations.csv fixture
// ---------------------------------------------------------------------------

#[test]
fn invitations_fixture() {
    let (stats, sink) = run(&imp(), &fixture("linkedin/Invitations.csv"));
    assert_eq!(stats.records_read, 2);
    assert_eq!(sink.interactions.len(), 2);
    assert_no_sentinel(&sink);
    let out = &sink.interactions[0];
    assert_eq!(out.kind, InteractionKind::Connection);
    assert_eq!(out.subject.as_deref(), Some("invitation"));
    assert_eq!(out.dir_hint, Some(Direction::Out));
    assert_eq!(out.at, utc(2026, 9, 15, 10, 0, 0));
    assert_eq!(
        out.external_id.as_deref(),
        Some("inv:outgoing:greta-solberg:9/15/26, 10:00 AM")
    );
    assert_eq!(out.participants.len(), 1);
    assert_eq!(out.participants[0].role, Role::Subject);
    assert_eq!(
        out.participants[0].hints.name.as_deref(),
        Some("Greta Solberg")
    );
    assert_eq!(
        out.participants[0].handles[0].raw,
        "https://www.linkedin.com/in/greta-solberg"
    );
    let inc = &sink.interactions[1];
    assert_eq!(inc.dir_hint, Some(Direction::In));
    assert_eq!(inc.at, utc(2024, 8, 6, 15, 45, 0));
    assert_eq!(inc.participants[0].hints.name.as_deref(), Some("Lena Park"));
    assert_eq!(
        inc.participants[0].handles[0].raw,
        "https://www.linkedin.com/in/lena-park-design"
    );
    assert_eq!(
        stats.self_candidates,
        vec!["linkedin_url:owen-ryder-example".to_string()]
    );
}

// ---------------------------------------------------------------------------
// CSV edge cases ported from the v0.1 prototype
// ---------------------------------------------------------------------------

#[test]
fn bom_crlf_preamble_and_multiline_values() {
    let content = format!(
        "\u{feff}Notes:\r\n\"Some notes, with a comma\r\nand a second line\"\r\n\r\n{CONN_HEADER}\r\n\
         Ann,Example,https://www.linkedin.com/in/ann-example,,\"Acme \"\"Labs\"\"\",\"Head of\r\nThings\",01 Jan 2025\r\n"
    );
    let (stats, sink) = run_csv("Connections.csv", &content);
    assert_eq!(stats.records_read, 1);
    let p = &sink.interactions[0].participants[0];
    assert_eq!(p.hints.company.as_deref(), Some("Acme \"Labs\""));
    assert_eq!(p.hints.title.as_deref(), Some("Head of Things"));
    assert_eq!(sink.interactions[0].at, utc(2025, 1, 1, 0, 0, 0));
}

#[test]
fn header_aliases_e_mail_and_title() {
    let (_, sink) = run_csv(
        "Connections.csv",
        "First Name,Last Name,Profile URL,E-mail Address,Organization,Title,Connected On\n\
         Ann,Example,https://www.linkedin.com/in/ann-example/,ann@example.com,Acme,CTO,2025-01-01\n",
    );
    let p = &sink.interactions[0].participants[0];
    assert_eq!(
        p.handles[1],
        Handle::new(HandleKind::Email, "ann@example.com")
    );
    assert_eq!(p.hints.title.as_deref(), Some("CTO"));
    assert_eq!(p.hints.company.as_deref(), Some("Acme"));
    assert_eq!(
        sink.interactions[0].external_id.as_deref(),
        Some("conn:ann-example")
    );
    assert_eq!(sink.interactions[0].at, utc(2025, 1, 1, 0, 0, 0));
}

#[test]
fn short_rows_bad_dates_and_rows_without_handles() {
    let (stats, sink) = run_csv(
        "Connections.csv",
        &format!(
            "{CONN_HEADER}\n\
             Ann,Example\n\
             Bo,Example,https://www.linkedin.com/in/bo-example,,Acme,PM,someday\n\
             Cy,Example,,,Acme,PM,01 Jan 2025\n\
             Di,Example,https://www.linkedin.com/in/di-example,,Acme,PM,01 Jan 2025\n"
        ),
    );
    assert_eq!(stats.records_read, 4);
    assert_eq!(sink.interactions.len(), 1);
    assert_eq!(stats.skipped.get("malformed"), Some(&1));
    assert_eq!(stats.skipped.get("bad_date"), Some(&1));
    assert_eq!(stats.skipped.get("no_participants"), Some(&1));
    assert_eq!(
        sink.skips,
        vec![
            (SkipReason::Malformed, "row 1".to_string()),
            (SkipReason::BadDate, "row 2".to_string()),
            (SkipReason::NoParticipants, "row 3".to_string()),
        ]
    );
}

#[test]
fn unclosed_quote_keeps_rows_before_it() {
    let (stats, sink) = run_csv(
        "Connections.csv",
        &format!(
            "{CONN_HEADER}\n\
             Ann,Example,https://www.linkedin.com/in/ann-example,,Acme,PM,01 Jan 2025\n\
             Bo,Example,https://www.linkedin.com/in/bo-example,,\"Acme,PM,01 Jan 2025\n"
        ),
    );
    assert_eq!(sink.interactions.len(), 1);
    assert!(stats.skipped_total() >= 1);
}

#[test]
fn unknown_csv_is_wrong_format_naming_the_columns() {
    let (_dir, p) = temp_file("people.csv", b"name,email\nAnn,ann@example.com\n");
    let err = imp().import(&p, &mut MemorySink::default()).unwrap_err();
    match err {
        ImportError::WrongFormat { kind, reason } => {
            assert_eq!(kind, "linkedin_csv");
            assert!(reason.contains("First Name"), "{reason}");
        }
        other => panic!("unexpected {other}"),
    }
}

#[test]
fn preamble_longer_than_ten_lines_is_rejected() {
    let mut s = String::from("Notes:\n");
    for i in 0..12 {
        s.push_str(&format!("note line {i}\n"));
    }
    s.push_str(CONN_HEADER);
    s.push('\n');
    let (_dir, p) = temp_file("Connections.csv", s.as_bytes());
    assert!(imp().import(&p, &mut MemorySink::default()).is_err());
}

// ---------------------------------------------------------------------------
// Detection and labels
// ---------------------------------------------------------------------------

#[test]
fn detect_levels() {
    let i = imp();
    for f in [
        "linkedin/Connections.csv",
        "linkedin/messages.csv",
        "linkedin/Invitations.csv",
    ] {
        assert_eq!(i.detect(&fixture(f)), Detection::Strong, "{f}");
    }
    let (_d, p) = temp_file("Connections.csv", b"garbage\n");
    assert_eq!(i.detect(&p), Detection::Weak);
    let (_d2, p2) = temp_file("other.csv", b"name,email\n");
    assert_eq!(i.detect(&p2), Detection::No);
    assert_eq!(i.detect(&fixture("vcf/contacts.vcf")), Detection::No);
    assert_eq!(i.detect(&fixture("mbox/small.mbox")), Detection::No);
}

#[test]
fn default_label_uses_export_folder_date_else_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let export = dir.path().join("Basic_LinkedInDataExport_10-15-2026");
    std::fs::create_dir(&export).unwrap();
    let p = export.join("Connections.csv");
    std::fs::write(&p, CONN_HEADER).unwrap();
    assert_eq!(
        imp().default_label(&p).as_deref(),
        Some("linkedin 2026-10-15")
    );
    let zip = dir.path().join("Basic_LinkedInDataExport_03-04-2025.zip");
    assert_eq!(
        imp().default_label(&zip).as_deref(),
        Some("linkedin 2025-03-04")
    );
    let (_d, plain) = temp_file("Connections.csv", CONN_HEADER.as_bytes());
    let label = imp().default_label(&plain).unwrap();
    let re = regex::Regex::new(r"^linkedin \d{4}-\d{2}-\d{2}$").unwrap();
    assert!(re.is_match(&label), "{label}");
}

#[test]
fn golden_fixture_stats() {
    for (name, file) in [
        ("linkedin_connections_stats", "linkedin/Connections.csv"),
        ("linkedin_messages_stats", "linkedin/messages.csv"),
        ("linkedin_invitations_stats", "linkedin/Invitations.csv"),
    ] {
        let (mut stats, _) = run(&imp(), &fixture(file));
        stats.elapsed_ms = 0; // wall time varies
        insta::assert_json_snapshot!(name, stats);
    }
}

// ---------------------------------------------------------------------------
// The export .zip LinkedIn emails (Complete_/Basic_LinkedInDataExport_*.zip)
// ---------------------------------------------------------------------------

/// Zip `(name, bytes)` entries into `dir/name` (deflated).
fn zip_entries(
    dir: &std::path::Path,
    name: &str,
    entries: &[(&str, Vec<u8>)],
) -> std::path::PathBuf {
    use std::io::Write;
    let p = dir.join(name);
    let f = std::fs::File::create(&p).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (n, b) in entries {
        z.start_file(*n, o).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap();
    p
}

/// The three synthetic CSVs plus files the importer must ignore, zipped
/// like LinkedIn's "larger data archive".
fn complete_export(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let read = |f: &str| std::fs::read(fixture(&format!("linkedin/{f}"))).unwrap();
    zip_entries(
        dir,
        name,
        &[
            (
                "Profile.csv",
                b"First Name,Last Name\nOwen,Ryder\n".to_vec(),
            ),
            ("Invitations.csv", read("Invitations.csv")),
            ("messages.csv", read("messages.csv")),
            ("Connections.csv", read("Connections.csv")),
            ("Rich_Media.csv", format!("x,{SENTINEL}\n").into_bytes()),
            ("Jobs/Job Applications.csv", b"a,b\n".to_vec()),
        ],
    )
}

#[test]
fn complete_export_zip_imports_all_three_csvs_as_one_source() {
    let dir = tempfile::tempdir().unwrap();
    let zip = complete_export(dir.path(), "Complete_LinkedInDataExport_10-20-2026.zip");
    assert_eq!(imp().detect(&zip), Detection::Strong);
    let (stats, sink) = run(&imp(), &zip);

    // Same records as importing the three files one by one (the messages
    // join to Connections works inside the zip too).
    let mut want = Vec::new();
    let mut read = 0;
    for f in ["Connections.csv", "messages.csv", "Invitations.csv"] {
        let (st, s) = run(&imp(), &fixture(&format!("linkedin/{f}")));
        read += st.records_read;
        want.extend(s.interactions);
    }
    assert_eq!(sink.interactions, want);
    assert_eq!(stats.records_read, read);
    assert_eq!(
        stats.self_candidates,
        vec!["linkedin_url:owen-ryder-example".to_string()]
    );
    assert!(stats.bytes > 0);
    assert_no_sentinel(&sink);
    assert_eq!(
        imp().default_label(&zip).as_deref(),
        Some("linkedin 2026-10-20")
    );
}

#[test]
fn double_suffixed_and_nested_export_zip_still_imports() {
    let dir = tempfile::tempdir().unwrap();
    let read = |f: &str| std::fs::read(fixture(&format!("linkedin/{f}"))).unwrap();
    let zip = zip_entries(
        dir.path(),
        "Complete_LinkedInDataExport_10-20-2026.zip.zip",
        &[(
            "Complete_LinkedInDataExport_10-20-2026/Connections.csv",
            read("Connections.csv"),
        )],
    );
    assert_eq!(imp().detect(&zip), Detection::Strong);
    let (stats, sink) = run(&imp(), &zip);
    assert_eq!(stats.records_read, 12);
    assert_eq!(sink.interactions.len(), 12);
    assert_eq!(
        imp().default_label(&zip).as_deref(),
        Some("linkedin 2026-10-20")
    );
    let reg = registry(&ImportOptions::default());
    assert_eq!(detect(&reg, &zip).unwrap().kind(), "linkedin_csv");
}

#[test]
fn basic_export_zip_without_connections_fails_with_actionable_message() {
    let dir = tempfile::tempdir().unwrap();
    let zip = zip_entries(
        dir.path(),
        "Basic_LinkedInDataExport_10-20-2026.zip",
        &[
            (
                "Profile.csv",
                b"First Name,Last Name\nOwen,Ryder\n".to_vec(),
            ),
            (
                "Email Addresses.csv",
                b"Email Address\nowen@example.com\n".to_vec(),
            ),
            ("Registration.csv", b"Registered At\n2020-01-01\n".to_vec()),
        ],
    );
    // Auto-detection still routes it here so the user gets the real reason.
    let reg = registry(&ImportOptions::default());
    assert_eq!(detect(&reg, &zip).unwrap().kind(), "linkedin_csv");
    let mut sink = MemorySink::default();
    let err = imp().import(&zip, &mut sink).unwrap_err();
    assert!(
        err.to_string().contains(
            "This is LinkedIn's Basic export; it has no Connections.csv. Request the larger \
             archive (Settings > Data privacy > Get a copy of your data > 'Download larger data \
             archive')."
        ),
        "{err}"
    );
    assert!(sink.interactions.is_empty());
}

#[test]
fn unrelated_zip_is_not_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let zip = zip_entries(dir.path(), "photos.zip", &[("a.csv", b"x,y\n".to_vec())]);
    assert_eq!(imp().detect(&zip), Detection::No);
    assert_eq!(
        imp().detect(&fixture("x/twitter-archive.zip")),
        Detection::No
    );
}

#[test]
fn oversized_zip_entry_aborts_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let zip = complete_export(dir.path(), "Complete_LinkedInDataExport_10-20-2026.zip");
    let small = imp().with_limits(64, 1 << 20);
    let mut sink = MemorySink::default();
    let err = small.import(&zip, &mut sink).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    let total = imp().with_limits(1 << 20, 64);
    let err = total.import(&zip, &mut MemorySink::default()).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
}
