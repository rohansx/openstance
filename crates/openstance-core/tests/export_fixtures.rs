//! P1-CORE-18 acceptance on the fixture graph: JSON export parses back to
//! the same people and fields; CSV has one row per exported person.

use std::path::{Path, PathBuf};

use openstance_core::export::{ExportFormat, ExportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::json::PersonDetail;
use openstance_core::{config, FixedClock, Graph, Providers};

fn graph() -> Graph {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join("fixtures/config.toml")).unwrap();
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let opts = openstance_import::ImportOptions::default();
    let mut g = Graph::open_in_memory(
        cfg,
        Providers::with_importers(openstance_import::registry(&opts)),
        Box::new(FixedClock(1_792_497_600)),
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

#[test]
fn export_json_round_trips_people_and_fields() {
    let g = graph();
    let mut out = Vec::new();
    let r = g
        .export(ExportFormat::Json, &ExportOptions::default(), &mut out)
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let people: Vec<PersonDetail> = serde_json::from_value(v["people"].clone()).unwrap();
    assert_eq!(r.people, 29, "30 people minus the bulk sender");
    assert_eq!(people.len(), 29);
    for p in &people {
        let fresh = g
            .who(&openstance_core::resolve::lookup::PersonKey::Id(
                p.person.id,
            ))
            .unwrap();
        assert_eq!(p.person.display_name, fresh.person.display_name);
        assert_eq!(p.person.title, fresh.person.title);
        assert_eq!(p.person.company, fresh.person.company);
        assert_eq!(p.person.score, fresh.person.score);
        assert_eq!(p.identities, fresh.identities);
    }
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("SENTINEL-BODY-7f3a"));
}

#[test]
fn export_csv_has_a_row_per_person_and_bulk_on_request() {
    let g = graph();
    let mut out = Vec::new();
    g.export(ExportFormat::Csv, &ExportOptions::default(), &mut out)
        .unwrap();
    let mut rdr = csv::Reader::from_reader(out.as_slice());
    assert_eq!(rdr.records().count(), 29);
    let mut out = Vec::new();
    let r = g
        .export(
            ExportFormat::Csv,
            &ExportOptions {
                include_bulk: true,
                ..ExportOptions::default()
            },
            &mut out,
        )
        .unwrap();
    assert_eq!(r.people, 30);
}
