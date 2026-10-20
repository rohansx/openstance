//! vcf importer (P1-IMP-04): vCard 2.1 / 3.0 / 4.0 from Apple and Google.

mod common;

use common::*;
use openstance_import::vcf::VcfImporter;
use openstance_import::*;

fn imp() -> VcfImporter {
    VcfImporter::new(ImportOptions::default())
}

fn run_str(content: &str) -> (ImportStats, MemorySink) {
    let (_dir, p) = temp_file("t.vcf", content.as_bytes());
    run(&imp(), &p)
}

fn by_record<'a>(sink: &'a MemorySink, record: &str) -> Vec<&'a RawIdentity> {
    sink.identities
        .iter()
        .filter(|i| i.record == record)
        .collect()
}

// ---------------------------------------------------------------------------
// The committed fixture
// ---------------------------------------------------------------------------

#[test]
fn fixture_counts_and_no_sentinel() {
    let (stats, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    assert_eq!(stats.records_read, 11);
    assert_eq!(stats.skipped_total(), 0);
    assert!(sink.skips.is_empty());
    assert!(sink.interactions.is_empty());
    // 11 cards, 11 distinct records.
    let mut records: Vec<_> = sink.identities.iter().map(|i| i.record.clone()).collect();
    records.dedup();
    assert_eq!(records.len(), 11);
    assert_no_sentinel(&sink);
}

#[test]
fn fixture_emails_and_phones() {
    let (_, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    let emails = handles(&sink, HandleKind::Email);
    assert_eq!(
        emails,
        [
            "priya@northwind.example",
            "lena.park@mail.test",
            "ines.q@mail.test",
            "jon.quillby@brightwater.example",
            "mira@tailspin.example",
            "mira.c@mail.test",
            "kofi@tailspin.example",
            "beatriz@mail.test",
            "lars@tailspin.example",
            "jose.garcia@tailspin.example",
            "hannah@contoso.example",
        ]
    );
    let phones = handles(&sink, HandleKind::Phone);
    assert_eq!(
        phones,
        ["+1 (201) 555-0142", "+1 201-555-0177", "+1-201-555-0188"]
    );
}

#[test]
fn fixture_priya_card_hints_rev_and_uid() {
    let (_, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    let ids = by_record(&sink, "8F1A0C44-0000-4000-8000-000000000001");
    assert_eq!(ids.len(), 2, "email + phone");
    for i in &ids {
        assert_eq!(i.hints.name.as_deref(), Some("Priya Raman"));
        assert_eq!(i.hints.company.as_deref(), Some("Northwind"));
        assert_eq!(i.hints.title.as_deref(), Some("VP Engineering"));
        assert_eq!(i.hints.location.as_deref(), Some("Bengaluru, KA"));
        assert_eq!(i.observed_at, Some(utc(2026, 3, 1, 12, 0, 0)));
    }
}

#[test]
fn fixture_v4_card_two_emails_and_tel_uri_share_a_record() {
    let (_, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    let ids = by_record(&sink, "urn:uuid:8f1a0c44-0000-4000-8000-000000000006");
    let kinds: Vec<_> = ids.iter().map(|i| i.handle.kind).collect();
    assert_eq!(
        kinds,
        [HandleKind::Email, HandleKind::Email, HandleKind::Phone]
    );
    assert_eq!(ids[2].handle.raw, "+1-201-555-0188");
    assert_eq!(ids[0].hints.location.as_deref(), Some("Lisbon"));
    assert_eq!(ids[0].observed_at, Some(utc(2025, 11, 20, 8, 30, 0)));
}

#[test]
fn fixture_phone_only_card() {
    let (_, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    let ids = by_record(&sink, "8F1A0C44-0000-4000-8000-000000000005");
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].handle.kind, HandleKind::Phone);
    assert_eq!(ids[0].hints.name.as_deref(), Some("Rufus Pennyworth"));
    assert_eq!(ids[0].observed_at, None);
}

#[test]
fn fixture_quoted_printable_name_is_decoded() {
    let (_, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    let ids = by_record(&sink, "8F1A0C44-0000-4000-8000-000000000010");
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].hints.name.as_deref(), Some("José García"));
}

#[test]
fn fixture_folded_email_and_prefix_name() {
    let (_, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    let ids = by_record(&sink, "8F1A0C44-0000-4000-8000-000000000011");
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].handle.raw, "hannah@contoso.example");
    assert_eq!(ids[0].hints.name.as_deref(), Some("Dr. Hannah O'Neil"));
    assert_eq!(ids[0].hints.company.as_deref(), Some("Contoso"));
    assert_eq!(ids[0].hints.title.as_deref(), Some("Research Lead"));
}

#[test]
fn fixture_photo_is_skipped_and_grouped_email_read() {
    let (_, sink) = run(&imp(), &fixture("vcf/contacts.vcf"));
    let kofi = by_record(&sink, "8F1A0C44-0000-4000-8000-000000000007");
    assert_eq!(kofi.len(), 1);
    assert_eq!(kofi[0].handle.raw, "kofi@tailspin.example");
    let lars = by_record(&sink, "8F1A0C44-0000-4000-8000-000000000009");
    assert_eq!(lars.len(), 1);
    assert_eq!(lars[0].handle.raw, "lars@tailspin.example");
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn name_only_card_emits_name_local() {
    let (stats, sink) = run_str("BEGIN:VCARD\nVERSION:3.0\nFN:Solo Example\nEND:VCARD\n");
    assert_eq!(stats.records_read, 1);
    assert_eq!(sink.identities.len(), 1);
    assert_eq!(sink.identities[0].handle.kind, HandleKind::NameLocal);
    assert_eq!(sink.identities[0].handle.raw, "vcf|Solo Example");
    assert_eq!(sink.identities[0].record, "card:1");
}

#[test]
fn empty_card_is_malformed() {
    let (stats, sink) = run_str(
        "BEGIN:VCARD\nVERSION:3.0\nNOTE:nothing here\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nEMAIL:a@example.com\nEND:VCARD\n",
    );
    assert_eq!(stats.records_read, 2);
    assert_eq!(stats.skipped.get("malformed"), Some(&1));
    assert_eq!(
        sink.skips,
        vec![(SkipReason::Malformed, "card 1".to_string())]
    );
    assert_eq!(sink.identities.len(), 1);
    assert_eq!(sink.identities[0].record, "card:2");
}

#[test]
fn n_is_reassembled_when_fn_is_missing() {
    let (_, sink) =
        run_str("BEGIN:VCARD\nVERSION:3.0\nN:Doe;Jane;Q;;\nEMAIL:j@example.com\nEND:VCARD\n");
    assert_eq!(sink.identities[0].hints.name.as_deref(), Some("Jane Doe"));
}

#[test]
fn social_urls_become_handles_other_urls_ignored() {
    let (_, sink) = run_str(concat!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Web Example\r\n",
        "item1.URL:https://www.linkedin.com/in/web-example/\r\n",
        "item1.X-ABLabel:LinkedIn\r\n",
        "URL:https://example.com/blog\r\n",
        "X-SOCIALPROFILE;type=twitter:https://twitter.com/webexample\r\n",
        "X-SOCIALPROFILE;type=instagram:https://www.instagram.com/web.example/\r\n",
        "URL:https://x.com/webex2\r\n",
        "X-SOCIALPROFILE;type=twitter;x-user=plainhandle:x-apple:plainhandle\r\n",
        "END:VCARD\r\n"
    ));
    let got: Vec<_> = sink
        .identities
        .iter()
        .map(|i| (i.handle.kind, i.handle.raw.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (
                HandleKind::LinkedinUrl,
                "https://www.linkedin.com/in/web-example/"
            ),
            (HandleKind::XHandle, "webexample"),
            (HandleKind::IgHandle, "web.example"),
            (HandleKind::XHandle, "webex2"),
            (HandleKind::XHandle, "plainhandle"),
        ]
    );
}

#[test]
fn quoted_printable_soft_line_breaks_and_latin1_charset() {
    let (_, sink) = run_str(concat!(
        "BEGIN:VCARD\r\nVERSION:2.1\r\n",
        "FN;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:Ren=C3=A9e =\r\n",
        "L=C3=B3pez\r\n",
        "ORG;CHARSET=ISO-8859-1;QUOTED-PRINTABLE:Caf=E9 Example;Kitchen\r\n",
        "TEL;CELL:+1 201 555 0100\r\n",
        "END:VCARD\r\n"
    ));
    assert_eq!(sink.identities.len(), 1);
    let h = &sink.identities[0].hints;
    assert_eq!(h.name.as_deref(), Some("Renée López"));
    assert_eq!(h.company.as_deref(), Some("Café Example"));
}

#[test]
fn escaped_values_are_unescaped() {
    let (_, sink) = run_str(concat!(
        "BEGIN:VCARD\nVERSION:3.0\nFN:Escaped Example\n",
        "ORG:Acme\\, Inc.;Dept\n",
        "TITLE:Head of R&D\\, EMEA\n",
        "EMAIL:mailto:esc@example.com\n",
        "END:VCARD\n"
    ));
    let h = &sink.identities[0].hints;
    assert_eq!(h.company.as_deref(), Some("Acme, Inc."));
    assert_eq!(h.title.as_deref(), Some("Head of R&D, EMEA"));
    assert_eq!(sink.identities[0].handle.raw, "esc@example.com");
}

#[test]
fn tab_folding_lowercase_keywords_and_tel_uri_extension() {
    let (_, sink) = run_str(concat!(
        "begin:vcard\nversion:4.0\nfn:Fold\n\tExample\n",
        "tel;value=uri:tel:+1-201-555-0101;ext=42\n",
        "end:vcard\n"
    ));
    assert_eq!(sink.identities.len(), 1);
    assert_eq!(
        sink.identities[0].hints.name.as_deref(),
        Some("FoldExample")
    );
    assert_eq!(sink.identities[0].handle.raw, "+1-201-555-0101");
}

#[test]
fn unterminated_card_is_malformed_and_bom_tolerated() {
    let (stats, sink) = run_str("\u{feff}BEGIN:VCARD\nVERSION:3.0\nEMAIL:a@example.com\nEND:VCARD\nBEGIN:VCARD\nEMAIL:b@example.com\n");
    assert_eq!(stats.records_read, 2);
    assert_eq!(sink.identities.len(), 1);
    assert_eq!(
        sink.skips,
        vec![(SkipReason::Malformed, "card 2".to_string())]
    );
}

#[test]
fn rev_formats() {
    let (_, sink) = run_str(concat!(
        "BEGIN:VCARD\nVERSION:3.0\nEMAIL:a@example.com\nREV:2024-05-06T07:08:09Z\nEND:VCARD\n",
        "BEGIN:VCARD\nVERSION:3.0\nEMAIL:b@example.com\nREV:20240506\nEND:VCARD\n",
        "BEGIN:VCARD\nVERSION:3.0\nEMAIL:c@example.com\nREV:garbage\nEND:VCARD\n",
    ));
    let revs: Vec<_> = sink.identities.iter().map(|i| i.observed_at).collect();
    assert_eq!(
        revs,
        [
            Some(utc(2024, 5, 6, 7, 8, 9)),
            Some(utc(2024, 5, 6, 0, 0, 0)),
            None
        ]
    );
}

#[test]
fn non_vcard_file_is_wrong_format() {
    let (_dir, p) = temp_file("t.vcf", b"hello world\n");
    let err = imp().import(&p, &mut MemorySink::default()).unwrap_err();
    assert!(
        matches!(err, ImportError::WrongFormat { kind: "vcf", .. }),
        "{err}"
    );
}

#[test]
fn detect_levels() {
    let i = imp();
    assert_eq!(i.detect(&fixture("vcf/contacts.vcf")), Detection::Strong);
    let (_d, p) = temp_file("x.vcard", b"nothing");
    assert_eq!(i.detect(&p), Detection::Weak);
    let (_d3, p3) = temp_file("x.txt", "\u{feff}\r\n BEGIN:VCARD\r\n".as_bytes());
    assert_eq!(i.detect(&p3), Detection::Strong);
    assert_eq!(
        i.detect(&fixture("does/not/exist.vcf")),
        Detection::No,
        "I/O errors are No"
    );
    assert_eq!(i.detect(&fixture("mbox/small.mbox")), Detection::No);
}
