//! P1-CORE-05: the staging writer (`GraphSink`, `stage_file`).
//! All data is synthetic (`*.example`, 555-01xx numbers).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use openstance_core::db::{migrate, open_in_memory};
use openstance_core::import::{
    Detection, Handle, HandleKind, Hints, ImportError, ImportStats, Importer, InteractionKind,
    Participant, RawIdentity, RawInteraction, Role, Sink, SkipReason,
};
use openstance_core::ingest::{
    file_hash, find_source_by_hash, finish_source, stage_file, GraphSink, ImportRequest,
    SinkOptions, StageOutcome,
};
use openstance_core::{Config, FixedClock, Graph, Providers};
use rusqlite::Connection;

fn conn() -> Connection {
    let mut c = open_in_memory().unwrap();
    migrate(&mut c, None, 0).unwrap();
    c.execute(
        "INSERT INTO source(id, kind, imported_at) VALUES (1, 'mbox', 0)",
        [],
    )
    .unwrap();
    c
}

fn email(raw: &str) -> Handle {
    Handle::new(HandleKind::Email, raw)
}

fn participant(handles: Vec<Handle>, role: Role, name: Option<&str>) -> Participant {
    Participant {
        handles,
        role,
        hints: Hints {
            name: name.map(str::to_string),
            ..Hints::default()
        },
    }
}

fn message(id: Option<&str>, participants: Vec<Participant>) -> RawInteraction {
    RawInteraction {
        kind: InteractionKind::Email,
        at: 1_700_000_000,
        dir_hint: None,
        subject: Some("Quarterly sync".into()),
        external_id: id.map(str::to_string),
        thread_key: Some("t1".into()),
        list_unsub: false,
        participants,
    }
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn opts() -> SinkOptions {
    SinkOptions {
        default_region: Some("US".into()),
        importer_kind: "mbox".into(),
    }
}

#[test]
fn interaction_writes_rows_with_roles_slots_and_hints() {
    let c = conn();
    let mut sink = GraphSink::new(&c, 1, opts(), 99);
    sink.interaction(message(
        Some("m1@example.com"),
        vec![
            participant(
                vec![email("Jane.Doe@Example.com")],
                Role::From,
                Some("Jane Doe"),
            ),
            participant(vec![email("owner@example.com")], Role::To, None),
            participant(vec![email("not an email")], Role::Cc, Some("Ghost")),
        ],
    ));
    let rep = sink.finish().unwrap();
    assert_eq!(rep.interactions_new, 1);
    assert_eq!(rep.identities_new, 2);
    assert_eq!(rep.identities_seen, 2);
    assert_eq!(rep.skipped.get("invalid_handle"), Some(&1));
    assert_eq!(rep.touched.identities.len(), 2);

    let (kind, ext, subject, thread): (String, String, String, String) = c
        .query_row(
            "SELECT kind, external_id, subject, thread_key FROM interaction",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (
            kind.as_str(),
            ext.as_str(),
            subject.as_str(),
            thread.as_str()
        ),
        ("email", "m1@example.com", "Quarterly sync", "t1")
    );
    let rows: Vec<(String, String, i64, Option<String>)> = c
        .prepare(
            "SELECT i.value, ip.role, ip.slot, ip.name_hint FROM interaction_person ip
             JOIN identity i ON i.id = ip.identity_id ORDER BY ip.slot",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "jane.doe@example.com".to_string(),
                "from".to_string(),
                0,
                Some("Jane Doe".to_string())
            ),
            ("owner@example.com".to_string(), "to".to_string(), 1, None),
        ]
    );
    // Identities are raw: source recorded, nothing derived yet.
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM identity WHERE source_id = 1 AND person_id IS NULL"
        ),
        2
    );
}

#[test]
fn interaction_without_valid_participants_is_skipped() {
    let c = conn();
    let mut sink = GraphSink::new(&c, 1, opts(), 99);
    sink.interaction(message(
        Some("m1"),
        vec![participant(
            vec![email("undisclosed-recipients:;")],
            Role::To,
            None,
        )],
    ));
    let rep = sink.finish().unwrap();
    assert_eq!(rep.interactions_new, 0);
    assert_eq!(rep.skipped.get("no_participants"), Some(&1));
    assert_eq!(count(&c, "SELECT count(*) FROM interaction"), 0);
}

#[test]
fn identities_sharing_a_record_become_one_contact_slot() {
    let c = conn();
    let mut sink = GraphSink::new(
        &c,
        1,
        SinkOptions {
            default_region: Some("US".into()),
            importer_kind: "vcf".into(),
        },
        500,
    );
    let hints = Hints {
        name: Some("Mira Castellanos".into()),
        company: Some("Tailspin".into()),
        ..Hints::default()
    };
    for (h, rec, at) in [
        (email("mira.c@mail.example"), "card-1", Some(400)),
        (
            Handle::new(HandleKind::Phone, "(201) 555-0188"),
            "card-1",
            None,
        ),
        (email("solo@example.com"), "card-2", None),
        (email("mira@tailspin.example"), "card-1", None),
    ] {
        sink.identity(RawIdentity {
            handle: h,
            hints: hints.clone(),
            record: rec.into(),
            observed_at: at,
        });
    }
    let rep = sink.finish().unwrap();
    assert_eq!(rep.interactions_new, 2);
    let contacts: Vec<(String, i64, i64)> = c
        .prepare(
            "SELECT i.external_id, i.at, count(*) FROM interaction i
             JOIN interaction_person ip ON ip.interaction_id = i.id
             WHERE i.kind = 'contact' AND ip.role = 'subject' AND ip.slot = 0
             GROUP BY i.id ORDER BY i.id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        contacts,
        vec![
            ("contact:card-1".to_string(), 400, 3),
            ("contact:card-2".to_string(), 500, 1),
        ]
    );
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM identity WHERE kind = 'phone' AND value = '+12015550188'"
        ),
        1
    );
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM interaction_person WHERE company_hint = 'Tailspin'"
        ),
        4
    );
}

#[test]
fn name_only_record_gets_a_scoped_name_local_identity() {
    let c = conn();
    let mut sink = GraphSink::new(
        &c,
        1,
        SinkOptions {
            default_region: None,
            importer_kind: "vcf".into(),
        },
        500,
    );
    sink.identity(RawIdentity {
        handle: Handle::new(HandleKind::NameLocal, "Rufus Example"),
        hints: Hints::default(),
        record: "c".into(),
        observed_at: None,
    });
    sink.finish().unwrap();
    assert_eq!(
        count(&c, "SELECT count(*) FROM identity WHERE kind = 'name_local' AND value = 'vcf|rufus example'"),
        1
    );
}

#[test]
fn dedupe_within_source_and_across_sources_for_email() {
    let c = conn();
    c.execute(
        "INSERT INTO source(id, kind, imported_at) VALUES (2, 'mbox', 0)",
        [],
    )
    .unwrap();
    let p = || vec![participant(vec![email("a@example.com")], Role::From, None)];
    let mut sink = GraphSink::new(&c, 1, opts(), 0);
    sink.interaction(message(Some("dup@example.com"), p()));
    sink.interaction(message(Some("dup@example.com"), p()));
    let rep = sink.finish().unwrap();
    assert_eq!((rep.interactions_new, rep.interactions_dup), (1, 1));

    // Layer 3: the same Message-ID in another source is a duplicate too...
    let mut sink = GraphSink::new(&c, 2, opts(), 0);
    sink.interaction(message(Some("dup@example.com"), p()));
    // ...but DMs are not deduped globally.
    let mut dm = message(Some("dup@example.com"), p());
    dm.kind = InteractionKind::Dm;
    sink.interaction(dm);
    let rep = sink.finish().unwrap();
    assert_eq!((rep.interactions_new, rep.interactions_dup), (1, 1));
    assert_eq!(count(&c, "SELECT count(*) FROM interaction"), 2);
}

#[test]
fn synthesized_external_ids_are_stable_hashes() {
    let c = conn();
    let p = || {
        vec![participant(
            vec![email("B@example.com"), email("a@example.com")],
            Role::Sender,
            None,
        )]
    };
    let mut sink = GraphSink::new(&c, 1, opts(), 0);
    let mut m = message(None, p());
    m.kind = InteractionKind::Dm;
    sink.interaction(m.clone());
    sink.interaction(m);
    let rep = sink.finish().unwrap();
    assert_eq!((rep.interactions_new, rep.interactions_dup), (1, 1));
    let ext: String = c
        .query_row("SELECT external_id FROM interaction", [], |r| r.get(0))
        .unwrap();
    assert!(ext.starts_with("syn:"), "{ext}");
    assert_eq!(ext.len(), 4 + 16);
    assert!(ext[4..].bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn subjects_are_truncated_and_control_characters_stripped() {
    let c = conn();
    let mut sink = GraphSink::new(&c, 1, opts(), 0);
    let mut m = message(
        Some("m"),
        vec![participant(vec![email("a@example.com")], Role::From, None)],
    );
    m.subject = Some(format!("Hi\u{0}\u{7}there{}", "é".repeat(600)));
    sink.interaction(m);
    sink.finish().unwrap();
    let subject: String = c
        .query_row("SELECT subject FROM interaction", [], |r| r.get(0))
        .unwrap();
    assert!(subject.starts_with("Hithere"));
    assert_eq!(subject.chars().count(), 512);
}

#[test]
fn importer_skips_are_counted() {
    let c = conn();
    let mut sink = GraphSink::new(&c, 1, opts(), 0);
    sink.skip(SkipReason::BadDate, "message 3");
    let rep = sink.finish().unwrap();
    assert_eq!(rep.skipped.get("bad_date"), Some(&1));
}

#[test]
fn file_hash_is_blake3_of_the_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.txt");
    std::fs::write(&p, b"hello").unwrap();
    let h = file_hash(&p).unwrap();
    assert_eq!(h, format!("blake3:{}", blake3::hash(b"hello").to_hex()));
    assert!(file_hash(&dir.path().join("missing")).is_err());
}

// ---------------------------------------------------------------------------
// stage_file and Graph::import
// ---------------------------------------------------------------------------

/// A toy importer: each line `email,name` is one inbound email; a line
/// `FAIL` returns an error, `PANIC` panics.
struct Lines;

impl Importer for Lines {
    fn kind(&self) -> &'static str {
        "mbox"
    }
    fn detect(&self, _path: &Path) -> Detection {
        Detection::Strong
    }
    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        let mut stats = ImportStats::default();
        for (i, line) in std::fs::read_to_string(path)?.lines().enumerate() {
            stats.records_read += 1;
            match line {
                "FAIL" => {
                    return Err(ImportError::Parse {
                        locator: format!("line {}", i + 1),
                        reason: "boom".into(),
                    })
                }
                "PANIC" => panic!("importer bug at line {}", i + 1),
                _ => {}
            }
            let (addr, name) = line.split_once(',').unwrap_or((line, ""));
            sink.interaction(RawInteraction {
                kind: InteractionKind::Email,
                at: 1_700_000_000 + i as i64,
                dir_hint: None,
                subject: Some(format!("line {i}")),
                external_id: Some(format!("{i}@{}", path.display())),
                thread_key: None,
                list_unsub: false,
                participants: vec![
                    participant(vec![email(addr)], Role::From, Some(name)),
                    participant(vec![email("owner@example.com")], Role::To, None),
                ],
            });
        }
        Ok(stats)
    }
}

fn graph() -> Graph {
    Graph::open_in_memory(
        Config::default(),
        Providers::with_importers(vec![Box::new(Lines)]),
        Box::new(FixedClock(1_790_812_800)),
    )
    .unwrap()
}

fn raw_only() -> ImportRequest {
    ImportRequest {
        no_resolve: true,
        ..ImportRequest::default()
    }
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn dump(c: &Connection) -> Vec<i64> {
    ["source", "identity", "interaction", "interaction_person"]
        .iter()
        .map(|t| count(c, &format!("SELECT count(*) FROM {t}")))
        .collect()
}

#[test]
fn stage_file_writes_a_labelled_source_and_merged_stats() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "a.mbox",
        "a@example.com,Ada Example\nb@example.com,Bo\n",
    );
    let c = conn();
    let out = stage_file(
        &c,
        &Lines,
        &p,
        &ImportRequest::default(),
        &Config::default(),
        1_790_812_800,
    )
    .unwrap();
    let StageOutcome::Staged(st) = out else {
        panic!("expected Staged")
    };
    assert_eq!(st.label, "mbox 2026-10-01");
    assert_eq!(st.stats.records_read, 2);
    assert_eq!(st.stats.interactions_new, 2);
    assert_eq!(st.stats.identities_new, 3);
    assert_eq!(st.touched.identities.len(), 3);
    let src = find_source_by_hash(&c, &file_hash(&p).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(src.id, st.source_id);
    assert_eq!(src.kind, "mbox");
    assert_eq!(src.imported_at, 1_790_812_800);
    finish_source(&c, st.source_id, &st.stats).unwrap();
    let json: String = c
        .query_row(
            "SELECT stats_json FROM source WHERE id = ?1",
            [st.source_id],
            |r| r.get(0),
        )
        .unwrap();
    let back: ImportStats = serde_json::from_str(&json).unwrap();
    assert_eq!(back, st.stats);
}

#[test]
fn stage_file_uses_the_requested_label() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.mbox", "a@example.com,Ada\n");
    let c = conn();
    let req = ImportRequest {
        label: Some("work gmail".into()),
        ..ImportRequest::default()
    };
    let StageOutcome::Staged(st) = stage_file(&c, &Lines, &p, &req, &Config::default(), 0).unwrap()
    else {
        panic!()
    };
    assert_eq!(st.label, "work gmail");
}

#[test]
fn importing_the_same_file_twice_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.mbox", "a@example.com,Ada Example\n");
    let mut g = graph();
    let first = g.import(std::slice::from_ref(&p), &raw_only()).unwrap();
    assert!(!first.sources[0].skipped_duplicate_file);
    let before = dump(g.conn());
    let again = g.import(&[p], &raw_only()).unwrap();
    assert!(again.sources[0].skipped_duplicate_file);
    assert_eq!(again.sources[0].id, first.sources[0].id);
    assert_eq!(dump(g.conn()), before);
}

#[test]
fn an_importer_error_rolls_the_whole_file_back() {
    let dir = tempfile::tempdir().unwrap();
    let ok = write(dir.path(), "ok.mbox", "a@example.com,Ada Example\n");
    let bad = write(dir.path(), "bad.mbox", "b@example.com,Bo Example\nFAIL\n");
    let mut g = graph();
    g.import(&[ok], &raw_only()).unwrap();
    let before = dump(g.conn());
    let err = g.import(&[bad], &raw_only()).unwrap_err();
    assert!(err.to_string().contains("import failed"), "{err}");
    assert_eq!(dump(g.conn()), before);
}

#[test]
fn an_importer_panic_rolls_the_whole_file_back() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let dir = tempfile::tempdir().unwrap();
    let bad = write(dir.path(), "bad.mbox", "b@example.com,Bo Example\nPANIC\n");
    let mut g = graph();
    let before = dump(g.conn());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CALLS.fetch_add(1, Ordering::SeqCst);
        g.import(std::slice::from_ref(&bad), &raw_only())
    }));
    assert!(r.is_err(), "the panic propagates to the caller");
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(dump(g.conn()), before);
}
