//! mbox importer (P1-IMP-02): headers only, streaming, Takeout quirks.

mod common;

use common::*;
use openstance_import::mbox::MboxImporter;
use openstance_import::*;

fn imp() -> MboxImporter {
    MboxImporter::new(ImportOptions::default())
}

fn run_mbox(content: &str) -> (ImportStats, MemorySink) {
    let (_dir, p) = temp_file("t.mbox", content.as_bytes());
    run(&imp(), &p)
}

fn run_mbox_with(opts: ImportOptions, content: &str) -> (ImportStats, MemorySink) {
    let (_dir, p) = temp_file("t.mbox", content.as_bytes());
    run(&MboxImporter::new(opts), &p)
}

/// A minimal message with the given extra headers.
fn msg(from_line: &str, headers: &str) -> String {
    format!("From {from_line}\n{headers}\nbody line SENTINEL-BODY-7f3a\n\n")
}

fn by_id<'a>(sink: &'a MemorySink, id: &str) -> &'a RawInteraction {
    sink.interactions
        .iter()
        .find(|i| i.external_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no interaction {id}"))
}

fn names(i: &RawInteraction) -> Vec<(Role, String, Option<String>)> {
    i.participants
        .iter()
        .map(|p| (p.role, p.handles[0].raw.clone(), p.hints.name.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// The committed fixture (numbers from fixtures/EXPECTED.md)
// ---------------------------------------------------------------------------

#[test]
fn fixture_stats_match_expected_md() {
    let (stats, sink) = run(&imp(), &fixture("mbox/small.mbox"));
    assert_eq!(stats.records_read, 177);
    assert_eq!(stats.skipped.get("bad_date"), Some(&1));
    assert_eq!(stats.skipped.get("unsupported"), Some(&1));
    assert_eq!(stats.skipped_total(), 2);
    // The duplicate Message-ID is emitted twice; the Sink dedupes it.
    assert_eq!(sink.interactions.len(), 175);
    let dup = "m0111.1789466400@example.com";
    assert_eq!(
        sink.interactions
            .iter()
            .filter(|i| i.external_id.as_deref() == Some(dup))
            .count(),
        2
    );
    assert_eq!(
        sink.skips,
        vec![
            (SkipReason::BadDate, "message 175".to_string()),
            (SkipReason::Unsupported, "message 176".to_string()),
        ]
    );
    assert_eq!(stats.bytes, 96890);
    assert!(sink.identities.is_empty());
}

#[test]
fn fixture_never_stores_bodies_or_skipped_senders() {
    let (_, sink) = run(&imp(), &fixture("mbox/small.mbox"));
    assert_no_sentinel(&sink);
    let all = all_strings(&sink).join("\n");
    for banned in [
        "ghost@example.invalid",
        "prizes@spam.example",
        "someone-in-a-body@example.invalid",
    ] {
        assert!(!all.contains(banned), "{banned} must not appear");
    }
}

#[test]
fn fixture_directions_lists_and_self_candidate() {
    let (stats, sink) = run(&imp(), &fixture("mbox/small.mbox"));
    let out = sink
        .interactions
        .iter()
        .filter(|i| i.dir_hint == Some(Direction::Out))
        .count();
    assert_eq!(out, 43, "Sent-labelled messages");
    // Every other fixture message carries a non-Sent Gmail label: it reached
    // the owner's mailbox, so it is hinted inbound (the only direction signal
    // when `[me]` is empty).
    assert!(sink
        .interactions
        .iter()
        .all(|i| i.dir_hint == Some(Direction::In) || i.dir_hint == Some(Direction::Out)));
    let unsub = sink.interactions.iter().filter(|i| i.list_unsub).count();
    assert_eq!(unsub, 60, "Digest Weekly issues");
    assert_eq!(
        stats.self_candidates,
        vec!["email:owner@example.com".to_string()]
    );
    let priya_from = sink
        .interactions
        .iter()
        .filter(|i| {
            i.participants[0].role == Role::From
                && i.participants[0].handles[0].raw == "priya@northwind.example"
        })
        .count();
    assert_eq!(priya_from, 52);
    assert!(sink
        .interactions
        .iter()
        .all(|i| i.kind == InteractionKind::Email));
}

#[test]
fn fixture_first_message_mapping() {
    let (_, sink) = run(&imp(), &fixture("mbox/small.mbox"));
    let m = &sink.interactions[0];
    assert_eq!(m.at, utc(2025, 1, 6, 9, 0, 0));
    assert_eq!(
        m.external_id.as_deref(),
        Some("m0001.1736154000@northwind.example")
    );
    assert_eq!(m.thread_key.as_deref(), Some("1810000000000001"));
    assert_eq!(m.subject.as_deref(), Some("Northwind platform sync #1"));
    assert_eq!(m.dir_hint, Some(Direction::In), "Inbox label");
    assert!(!m.list_unsub);
    assert_eq!(
        names(m),
        vec![
            (
                Role::From,
                "priya@northwind.example".into(),
                Some("Priya Raman".into())
            ),
            (
                Role::To,
                "owner@example.com".into(),
                Some("Owen Ryder".into())
            ),
        ]
    );
    let reply = &sink.interactions[1];
    assert_eq!(reply.dir_hint, Some(Direction::Out));
    assert_eq!(
        reply.subject.as_deref(),
        Some("Re: Northwind platform sync #1")
    );
}

#[test]
fn fixture_rfc2047_names_and_subjects() {
    let (_, sink) = run(&imp(), &fixture("mbox/small.mbox"));
    let kofi = sink
        .interactions
        .iter()
        .find(|i| i.participants[0].handles[0].raw == "kofi@tailspin.example")
        .unwrap();
    assert_eq!(
        kofi.participants[0].hints.name.as_deref(),
        Some("Kofi Mensah \u{1F680}")
    );
    assert_eq!(
        kofi.subject.as_deref(),
        Some("Devtools dinner \u{2013} Thursday")
    );
    let hannah = sink
        .interactions
        .iter()
        .find(|i| i.participants[0].handles[0].raw == "hannah@contoso.example")
        .unwrap();
    assert_eq!(
        hannah.participants[0].hints.name.as_deref(),
        Some("Hannah O'Neil")
    );
    let elena = sink
        .interactions
        .iter()
        .find(|i| i.participants[0].handles[0].raw == "elena@tailspin.example")
        .unwrap();
    assert_eq!(
        elena.participants[0].hints.name.as_deref(),
        Some("Voss, Elena")
    );
}

#[test]
fn fixture_cc_group_and_in_reply_to_threads() {
    let (_, sink) = run(&imp(), &fixture("mbox/small.mbox"));
    let marcus_cc = sink
        .interactions
        .iter()
        .filter(|i| {
            i.participants
                .iter()
                .any(|p| p.role == Role::Cc && p.handles[0].raw == "marcus@contoso.example")
        })
        .count();
    assert_eq!(marcus_cc, 3);
    let digest1 = by_id(&sink, "m0115.1748844000@digest.example");
    assert_eq!(
        digest1.participants.len(),
        1,
        "undisclosed-recipients:; yields nobody"
    );
    assert!(digest1.list_unsub);
    // No X-GM-THRID: own Message-ID, then References / In-Reply-To.
    let a = by_id(&sink, "m0089.1776159000@example.com");
    let b = by_id(&sink, "m0090.1776164400@mail.test");
    assert_eq!(
        a.thread_key.as_deref(),
        Some("m0089.1776159000@example.com")
    );
    assert_eq!(b.thread_key, a.thread_key);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

const BASIC: &str = "From: Ann Example <ann@example.com>\nTo: bo@example.com\nSubject: hi\n";

#[test]
fn date_falls_back_to_from_line_then_bad_date() {
    let content = [
        msg(
            "ann@example.com Mon Jan  6 09:00:00 2025",
            &format!("{BASIC}Message-ID: <a@x>\n"),
        ),
        msg(
            "1234567890@xxx Tue Jan 07 10:30:00 +0200 2025",
            &format!("{BASIC}Message-ID: <b@x>\nDate: nonsense\n"),
        ),
        msg(
            "ann@example.com garbage",
            &format!("{BASIC}Message-ID: <c@x>\n"),
        ),
    ]
    .concat();
    let (stats, sink) = run_mbox(&content);
    assert_eq!(stats.records_read, 3);
    assert_eq!(by_id(&sink, "a@x").at, utc(2025, 1, 6, 9, 0, 0));
    assert_eq!(by_id(&sink, "b@x").at, utc(2025, 1, 7, 8, 30, 0));
    assert_eq!(
        sink.skips,
        vec![(SkipReason::BadDate, "message 3".to_string())]
    );
}

#[test]
fn common_date_malformations() {
    let content = [
        msg(
            "x",
            &format!("{BASIC}Message-ID: <a@x>\nDate: 6 Jan 2025 09:00 +0000\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}Message-ID: <b@x>\nDate: Mon, 06 Jan 25 09:00:00 GMT\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}Message-ID: <c@x>\nDate: Mon, 6 Jan 2025 04:00:00 EST\n"),
        ),
    ]
    .concat();
    let (_, sink) = run_mbox(&content);
    assert_eq!(by_id(&sink, "a@x").at, utc(2025, 1, 6, 9, 0, 0));
    assert_eq!(by_id(&sink, "b@x").at, utc(2025, 1, 6, 9, 0, 0));
    assert_eq!(by_id(&sink, "c@x").at, utc(2025, 1, 6, 9, 0, 0));
}

#[test]
fn bulk_signals() {
    let d = "Date: Mon, 06 Jan 2025 09:00:00 +0000\n";
    let content = [
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <a@x>\nPrecedence: bulk\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <b@x>\nAuto-Submitted: auto-replied\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <c@x>\nAuto-Submitted: no\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <d@x>\nPrecedence: first-class\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <e@x>\nList-Unsubscribe: <mailto:u@example.com>\n"),
        ),
    ]
    .concat();
    let (_, sink) = run_mbox(&content);
    let flags: Vec<_> = sink.interactions.iter().map(|i| i.list_unsub).collect();
    assert_eq!(flags, [true, true, false, false, true]);
}

#[test]
fn spam_trash_and_chat_labels() {
    let d = "Date: Mon, 06 Jan 2025 09:00:00 +0000\n";
    let content = [
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <a@x>\nX-Gmail-Labels: Trash\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <b@x>\nX-Gmail-Labels: Inbox,\"Spam\"\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <c@x>\nX-Gmail-Labels: Chat\n"),
        ),
        msg(
            "x",
            &format!("{BASIC}{d}Message-ID: <d@x>\nX-Gmail-Labels: Important,Spammy-Newsletters\n"),
        ),
    ]
    .concat();
    let (stats, sink) = run_mbox(&content);
    assert_eq!(stats.skipped.get("unsupported"), Some(&3));
    assert_eq!(sink.interactions.len(), 1);
    let opts = ImportOptions {
        include_spam: true,
        ..Default::default()
    };
    let (stats, sink) = run_mbox_with(opts, &content);
    assert_eq!(
        stats.skipped.get("unsupported"),
        Some(&1),
        "Chat is never imported"
    );
    assert_eq!(sink.interactions.len(), 3);
}

#[test]
fn bcc_references_and_missing_message_id() {
    let d = "Date: Mon, 06 Jan 2025 09:00:00 +0000\n";
    let content = [
        msg(
            "x",
            &format!(
                "{BASIC}{d}Message-ID: <A@X>\nBcc: Secret <s@example.com>\nCc: c1@example.com, \"Two, C\" <c2@example.com>\nReferences: <Root@x> <mid@x>\nIn-Reply-To: <mid@x>\nX-Gmail-Labels: Sent\n"
            ),
        ),
        msg("x", &format!("{BASIC}{d}")),
    ]
    .concat();
    let (stats, sink) = run_mbox(&content);
    let a = &sink.interactions[0];
    assert_eq!(a.external_id.as_deref(), Some("a@x"));
    assert_eq!(a.thread_key.as_deref(), Some("root@x"));
    assert_eq!(
        names(a),
        vec![
            (
                Role::From,
                "ann@example.com".into(),
                Some("Ann Example".into())
            ),
            (Role::To, "bo@example.com".into(), None),
            (Role::Cc, "c1@example.com".into(), None),
            (Role::Cc, "c2@example.com".into(), Some("Two, C".into())),
            (Role::Bcc, "s@example.com".into(), Some("Secret".into())),
        ]
    );
    let b = &sink.interactions[1];
    assert_eq!(b.external_id, None);
    assert_eq!(b.thread_key, None);
    assert_eq!(
        stats.self_candidates,
        vec!["email:ann@example.com".to_string()]
    );
}

#[test]
fn messages_without_addresses_are_skipped() {
    let content = msg(
        "x",
        "Subject: nobody\nDate: Mon, 06 Jan 2025 09:00:00 +0000\nTo: undisclosed-recipients:;\n",
    );
    let (stats, sink) = run_mbox(&content);
    assert_eq!(stats.records_read, 1);
    assert!(sink.interactions.is_empty());
    assert_eq!(
        sink.skips,
        vec![(SkipReason::NoParticipants, "message 1".to_string())]
    );
}

#[test]
fn gmail_labels_hint_direction_and_unlabelled_mail_stays_unknown() {
    let content = [
        msg(
            "a@x.example Mon Jan 06 09:00:00 2025",
            "X-Gmail-Labels: Sent\nMessage-ID: <s@x>\nDate: Mon, 06 Jan 2025 09:00:00 +0000\nFrom: a@x.example\nTo: b@y.example\n",
        ),
        msg(
            "b@y.example Mon Jan 06 10:00:00 2025",
            "X-Gmail-Labels: Inbox,Important\nMessage-ID: <i@y>\nDate: Mon, 06 Jan 2025 10:00:00 +0000\nFrom: b@y.example\nTo: a@x.example\n",
        ),
        msg(
            "b@y.example Mon Jan 06 11:00:00 2025",
            "X-Gmail-Labels: Archived\nMessage-ID: <r@y>\nDate: Mon, 06 Jan 2025 11:00:00 +0000\nFrom: b@y.example\nTo: a@x.example\n",
        ),
        msg(
            "c@z.example Mon Jan 06 12:00:00 2025",
            "Message-ID: <u@z>\nDate: Mon, 06 Jan 2025 12:00:00 +0000\nFrom: c@z.example\nTo: a@x.example\n",
        ),
    ]
    .concat();
    let (_, sink) = run_mbox(&content);
    assert_eq!(by_id(&sink, "s@x").dir_hint, Some(Direction::Out));
    assert_eq!(by_id(&sink, "i@y").dir_hint, Some(Direction::In));
    assert_eq!(by_id(&sink, "r@y").dir_hint, Some(Direction::In));
    // A plain mbox (no Takeout labels) says nothing about the mailbox owner.
    assert_eq!(by_id(&sink, "u@z").dir_hint, None);
}

#[test]
fn body_from_lines_only_split_after_a_blank_line() {
    let content = format!(
        "From a Mon Jan  6 09:00:00 2025\n{BASIC}Message-ID: <a@x>\n\nline one\nFrom here on it is body\n>From escaped\n\nFrom b Mon Jan  6 10:00:00 2025\n{BASIC}Message-ID: <b@x>\n\nbody\n"
    );
    let (stats, sink) = run_mbox(&content);
    assert_eq!(stats.records_read, 2);
    assert_eq!(sink.interactions.len(), 2);
    assert_eq!(by_id(&sink, "b@x").at, utc(2025, 1, 6, 10, 0, 0));
}

#[test]
fn crlf_line_endings_and_folded_headers() {
    let content = "From a Mon Jan  6 09:00:00 2025\r\nFrom: Ann\r\n Example <ann@example.com>\r\nTo: bo@example.com\r\nSubject: a long\r\n\tsubject\r\nMessage-ID: <a@x>\r\nDate: Mon, 06 Jan 2025 09:00:00 +0000\r\n\r\nbody\r\n\r\nFrom b Mon Jan  6 10:00:00 2025\r\nFrom: bo@example.com\r\nMessage-ID: <b@x>\r\n\r\n";
    let (stats, sink) = run_mbox(content);
    assert_eq!(stats.records_read, 2);
    let a = by_id(&sink, "a@x");
    assert_eq!(a.participants[0].hints.name.as_deref(), Some("Ann Example"));
    assert_eq!(a.subject.as_deref(), Some("a long subject"));
    assert_eq!(by_id(&sink, "b@x").at, utc(2025, 1, 6, 10, 0, 0));
}

#[test]
fn oversized_header_is_skipped_and_parsing_resumes() {
    let huge = format!("X-Junk: {}\n", "a".repeat(300 * 1024));
    let content = [
        msg(
            "x",
            &format!("{BASIC}Date: Mon, 06 Jan 2025 09:00:00 +0000\n{huge}"),
        ),
        msg(
            "x",
            &format!("{BASIC}Date: Mon, 06 Jan 2025 09:00:00 +0000\nMessage-ID: <ok@x>\n"),
        ),
    ]
    .concat();
    let (stats, sink) = run_mbox(&content);
    assert_eq!(stats.records_read, 2);
    assert_eq!(
        sink.skips,
        vec![(SkipReason::HeaderTooLarge, "message 1".to_string())]
    );
    assert_eq!(sink.interactions.len(), 1);
}

#[test]
fn messages_over_the_size_cap_are_skipped() {
    let d = "Date: Mon, 06 Jan 2025 09:00:00 +0000\n";
    let content = format!(
        "From x\n{BASIC}{d}Message-ID: <big@x>\n\n{}\n\nFrom y\n{BASIC}{d}Message-ID: <small@x>\n\nok\n",
        "b".repeat(2000)
    );
    let opts = ImportOptions {
        max_message_bytes: 1000,
        ..Default::default()
    };
    let (_, sink) = run_mbox_with(opts, &content);
    assert_eq!(
        sink.skips,
        vec![(SkipReason::Unsupported, "message 1".to_string())]
    );
    assert_eq!(sink.interactions.len(), 1);
}

#[test]
fn bodies_option_is_not_supported_yet() {
    let (_dir, p) = temp_file("t.mbox", msg("x", BASIC).as_bytes());
    let opts = ImportOptions {
        email_bodies: true,
        ..Default::default()
    };
    let err = MboxImporter::new(opts)
        .import(&p, &mut MemorySink::default())
        .unwrap_err();
    assert!(matches!(err, ImportError::Unsupported(_)), "{err}");
    assert!(err.to_string().contains("later release"), "{err}");
}

#[test]
fn non_mbox_is_wrong_format() {
    let (_dir, p) = temp_file("t.mbox", b"\n\nhello\nFrom x\n");
    let err = imp().import(&p, &mut MemorySink::default()).unwrap_err();
    assert!(
        matches!(err, ImportError::WrongFormat { kind: "mbox", .. }),
        "{err}"
    );
}

#[test]
fn detect_levels() {
    let i = imp();
    assert_eq!(i.detect(&fixture("mbox/small.mbox")), Detection::Strong);
    let (_d, p) = temp_file("x.mbox", b"nothing to see");
    assert_eq!(i.detect(&p), Detection::Weak);
    let (_d2, p2) = temp_file(
        "Inbox",
        b"\r\nFrom a@b Mon Jan  6 09:00:00 2025\r\nmessage-id: <a@b>\r\n",
    );
    assert_eq!(i.detect(&p2), Detection::Strong);
    let (_d3, p3) = temp_file("notes.txt", b"From the desk of someone\nhello\n");
    assert_eq!(i.detect(&p3), Detection::No);
    assert_eq!(i.detect(&fixture("vcf/contacts.vcf")), Detection::No);
    assert_eq!(
        i.detect(&fixture("linkedin/Connections.csv")),
        Detection::No
    );
}

#[test]
fn huge_single_line_body_is_streamed_past() {
    let d = "Date: Mon, 06 Jan 2025 09:00:00 +0000\n";
    let mut content = format!("From x\n{BASIC}{d}Message-ID: <a@x>\n\n");
    content.push_str(&"z".repeat(5 * 1024 * 1024));
    content.push_str(&format!("\n\nFrom y\n{BASIC}{d}Message-ID: <b@x>\n\nok\n"));
    let (stats, sink) = run_mbox(&content);
    assert_eq!(stats.records_read, 2);
    assert_eq!(sink.interactions.len(), 2);
    assert_eq!(stats.bytes, content.len() as u64);
    assert_eq!(
        sink.last_progress,
        Some((content.len() as u64, Some(content.len() as u64)))
    );
}

#[test]
fn golden_fixture_stats() {
    let (mut stats, _) = run(&imp(), &fixture("mbox/small.mbox"));
    stats.elapsed_ms = 0; // wall time varies
    insta::assert_json_snapshot!("mbox_fixture_stats", stats);
}

/// Throughput check for the perf job: `cargo test --release -p
/// openstance-import --test mbox -- --ignored perf`. Generates ~256 MiB of
/// synthetic mail and requires >= 45 MiB/s (5 GB in under 2 minutes).
#[test]
#[ignore = "perf job only"]
fn perf_streaming_throughput() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.mbox");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let body = "Lorem ipsum synthetic body line for throughput testing only.\n".repeat(60);
    let mut n = 0u64;
    let mut written = 0usize;
    while written < 256 * 1024 * 1024 {
        n += 1;
        let m = format!(
            "From a{n}@example.com Mon Jan  6 09:00:00 2025\nFrom: Person {n} <p{n}@example.com>\nTo: owner@example.com\nSubject: hello {n}\nMessage-ID: <{n}@example.com>\nDate: Mon, 06 Jan 2025 09:00:00 +0000\n\n{body}\n"
        );
        written += m.len();
        f.write_all(m.as_bytes()).unwrap();
    }
    f.flush().unwrap();
    drop(f);
    let started = std::time::Instant::now();
    let mut sink = CountingSink::default();
    let stats = imp().import(&path, &mut sink).unwrap();
    let secs = started.elapsed().as_secs_f64();
    let mibs = written as f64 / (1024.0 * 1024.0) / secs;
    eprintln!("{n} messages, {mibs:.0} MiB/s");
    assert_eq!(stats.records_read, n);
    assert_eq!(sink.0, n);
    assert!(mibs >= 45.0, "{mibs:.1} MiB/s");
}

/// Counts interactions without keeping them (memory stays flat).
#[derive(Default)]
struct CountingSink(u64);

impl Sink for CountingSink {
    fn identity(&mut self, _r: RawIdentity) {}
    fn interaction(&mut self, _r: RawInteraction) {
        self.0 += 1;
    }
    fn skip(&mut self, _reason: SkipReason, _locator: &str) {}
}
