//! `POST /edits` (interfaces.md 3.2) on the fixture graph: every op becomes
//! override rows, the whole batch is one transaction, and the response
//! lists the people it touched.

use std::path::{Path, PathBuf};

use openstance_core::ingest::ImportRequest;
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::workspace::edits::{EditOp, EditsRequest};
use openstance_core::{config, Error, FixedClock, Graph, Providers};

const NOW: i64 = 1_792_497_600; // 2026-10-20T12:00:00Z

fn graph() -> Graph {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join("fixtures/config.toml")).unwrap();
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let opts = openstance_import::ImportOptions::default();
    let mut g = Graph::open_in_memory(
        cfg,
        Providers::with_importers(openstance_import::registry(&opts)),
        Box::new(FixedClock(NOW)),
    )
    .unwrap();
    let files: Vec<PathBuf> = [
        "fixtures/mbox/small.mbox",
        "fixtures/linkedin/Connections.csv",
        "fixtures/linkedin/messages.csv",
        "fixtures/linkedin/Invitations.csv",
        "fixtures/vcf/contacts.vcf",
    ]
    .iter()
    .map(|f| root.join(f))
    .collect();
    g.import(&files, &ImportRequest::default()).unwrap();
    g
}

fn id(g: &Graph, r: &str) -> i64 {
    g.who(&PersonKey::parse(r)).unwrap().person.id
}

fn ops(v: serde_json::Value) -> Vec<EditOp> {
    serde_json::from_value::<EditsRequest>(v).unwrap().ops
}

#[test]
fn the_documented_ops_parse() {
    let r: EditsRequest = serde_json::from_str(
        r#"{ "ops": [
          { "op": "set",    "person": 42, "field": "title", "value": "CTO" },
          { "op": "unset",  "person": 42, "field": "location" },
          { "op": "tag",    "person": 42, "value": "investor" },
          { "op": "untag",  "person": 42, "value": "devtools" },
          { "op": "note",   "person": 42, "value": "met at YC demo day" },
          { "op": "flag",   "person": 42, "field": "do_not_share", "value": true },
          { "op": "merge",  "a": 42, "b": 97 },
          { "op": "split",  "person": 42, "identities": [5531] },
          { "op": "unmerge","a": 42, "b": 97 },
          { "op": "confirm","person": "sam@example.com" },
          { "op": "reject", "a": 3, "b": "Sam Ito" }
        ] }"#,
    )
    .unwrap();
    assert_eq!(r.ops.len(), 11);
    assert!(
        serde_json::from_str::<EditsRequest>(r#"{"ops":[{"op":"explode","person":1}]}"#).is_err()
    );
}

#[test]
fn field_ops_apply_and_report_the_person() {
    let mut g = graph();
    let p = id(&g, "priya@northwind.example");
    let r = g
        .apply_edits(&ops(serde_json::json!({ "ops": [
            { "op": "set", "person": p, "field": "title", "value": "Chief Brewer" },
            { "op": "tag", "person": "priya@northwind.example", "value": "brewing" },
            { "op": "note", "person": p, "value": "likes short asks" },
            { "op": "flag", "person": p, "field": "no_enrich", "value": true }
        ]})))
        .unwrap();
    assert_eq!(r.v, 1);
    assert_eq!(r.applied, 4);
    assert_eq!(r.people.len(), 1);
    assert_eq!(r.people[0].id, p);
    assert_eq!(r.people[0].title.as_deref(), Some("Chief Brewer"));
    let d = g.who(&PersonKey::Id(p)).unwrap();
    assert!(d.person.tags.contains(&"brewing".to_string()));
    assert!(d.person.no_enrich);
    assert_eq!(d.notes.len(), 1);
    assert_eq!(
        d.person
            .field_sources
            .get("title")
            .and_then(|s| s.as_deref()),
        Some("override")
    );
}

#[test]
fn a_bad_op_rolls_back_the_whole_batch() {
    let mut g = graph();
    let p = id(&g, "priya@northwind.example");
    let before = g.who(&PersonKey::Id(p)).unwrap().person.title;
    let err = g
        .apply_edits(&ops(serde_json::json!({ "ops": [
            { "op": "set", "person": p, "field": "title", "value": "Should Not Stick" },
            { "op": "set", "person": p, "field": "shoe_size", "value": "44" }
        ]})))
        .unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "{err:?}");
    assert_eq!(g.who(&PersonKey::Id(p)).unwrap().person.title, before);
}

#[test]
fn an_empty_batch_is_a_bad_request() {
    let mut g = graph();
    assert!(matches!(g.apply_edits(&[]), Err(Error::BadRequest(_))));
}

#[test]
fn merge_then_unmerge() {
    let mut g = graph();
    let a = id(&g, "priya@northwind.example");
    let b = id(&g, "marcus@contoso.example");
    let r = g
        .apply_edits(&ops(
            serde_json::json!({ "ops": [ { "op": "merge", "a": a, "b": b } ]}),
        ))
        .unwrap();
    let kept = a.min(b);
    assert_eq!(
        r.people.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![kept]
    );
    assert_eq!(id(&g, "marcus@contoso.example"), kept);
    g.apply_edits(&ops(
        serde_json::json!({ "ops": [ { "op": "unmerge", "a": a, "b": b } ]}),
    ))
    .unwrap();
    assert_ne!(
        id(&g, "marcus@contoso.example"),
        id(&g, "priya@northwind.example")
    );
}

#[test]
fn confirm_and_split_a_flagged_merge() {
    let mut g = graph();
    let q = g.review().unwrap();
    assert!(q.flagged.len() >= 2);
    let first = &q.flagged[0];
    let r = g
        .apply_edits(&ops(serde_json::json!({ "ops": [
            { "op": "confirm", "person": first.person.id }
        ]})))
        .unwrap();
    assert_eq!(r.applied, 1);
    let after = g.review().unwrap();
    assert!(after.flagged.iter().all(|f| f.person.id != first.person.id));

    let second = &after.flagged[0];
    let moved: Vec<i64> = second.flagged_identities.iter().map(|i| i.id).collect();
    let r = g
        .apply_edits(&ops(serde_json::json!({ "ops": [
            { "op": "split", "person": second.person.id, "identities": moved }
        ]})))
        .unwrap();
    assert_eq!(r.people.len(), 2, "kept and moved sides");
}
