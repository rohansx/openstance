//! P1-IMP-05: every Phase 1 fixture is detected `Strong` by exactly one
//! importer; unknown files error with the supported kinds.

mod common;

use common::*;
use openstance_import::*;

const FIXTURES: &[(&str, &str)] = &[
    ("mbox/small.mbox", "mbox"),
    ("linkedin/Connections.csv", "linkedin_csv"),
    ("linkedin/messages.csv", "linkedin_csv"),
    ("linkedin/Invitations.csv", "linkedin_csv"),
    ("vcf/contacts.vcf", "vcf"),
];

#[test]
fn registry_order_is_mbox_linkedin_vcf() {
    let reg = registry(&ImportOptions::default());
    let kinds: Vec<_> = reg.iter().map(|i| i.kind()).collect();
    assert_eq!(kinds, ["mbox", "linkedin_csv", "vcf"]);
}

#[test]
fn detect_matrix_each_fixture_strong_for_exactly_one_importer() {
    let reg = registry(&ImportOptions::default());
    for (rel, want) in FIXTURES {
        let path = fixture(rel);
        let strong: Vec<_> = reg
            .iter()
            .filter(|i| i.detect(&path) == Detection::Strong)
            .map(|i| i.kind())
            .collect();
        assert_eq!(strong, [*want], "{rel}");
        for i in reg.iter().filter(|i| i.kind() != *want) {
            assert_eq!(i.detect(&path), Detection::No, "{} on {rel}", i.kind());
        }
        assert_eq!(detect(&reg, &path).unwrap().kind(), *want, "{rel}");
    }
}

#[test]
fn detection_ignores_misleading_extensions() {
    let reg = registry(&ImportOptions::default());
    // A vCard saved as .csv and an mbox saved as .vcf are still recognized.
    let vcard = std::fs::read(fixture("vcf/contacts.vcf")).unwrap();
    let (_d, p) = temp_file("contacts.csv", &vcard);
    assert_eq!(detect(&reg, &p).unwrap().kind(), "vcf");
    let mbox = std::fs::read(fixture("mbox/small.mbox")).unwrap();
    let (_d2, p2) = temp_file("mail.vcf", &mbox);
    assert_eq!(detect(&reg, &p2).unwrap().kind(), "mbox");
}

#[test]
fn weak_matches_are_used_when_nothing_is_strong() {
    let reg = registry(&ImportOptions::default());
    let (_d, p) = temp_file("archive.mbox", b"");
    assert_eq!(detect(&reg, &p).unwrap().kind(), "mbox");
    let (_d2, p2) = temp_file("Connections.csv", b"");
    assert_eq!(detect(&reg, &p2).unwrap().kind(), "linkedin_csv");
}

#[test]
fn unknown_file_lists_supported_kinds() {
    let reg = registry(&ImportOptions::default());
    let (_d, p) = temp_file("notes.txt", b"just some notes\n");
    let err = detect(&reg, &p).err().unwrap();
    assert_eq!(
        err.to_string(),
        "no importer recognizes this file; supported kinds: mbox, linkedin_csv, vcf; use --as <kind>"
    );
    let missing = fixture("does-not-exist.mbox");
    assert!(detect(&reg, &missing).is_err(), "I/O errors detect as No");
}

#[test]
fn by_kind_imports_whatever_detection_says() {
    let reg = registry(&ImportOptions::default());
    for (rel, want) in FIXTURES {
        let imp = by_kind(&reg, want).unwrap();
        let (stats, sink) = run(imp, &fixture(rel));
        assert!(stats.records_read > 0, "{rel}");
        assert_no_sentinel(&sink);
    }
}
