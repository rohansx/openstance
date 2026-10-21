//! P4-CORE-02 / P4-CORE-03 on real imports: two LinkedIn `Connections.csv`
//! exports, each its own `linkedin_csv` source. `accounts` reads a job
//! change as alumni; `watch` diffs the latest two exports with zero egress.
//! Every name and domain is fictional.

use std::path::{Path, PathBuf};

use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::playbook::{RunOptions, RunStatus};
use openstance_core::{Config, FixedClock, Graph, Providers};
use serde_json::{json, Value};

/// 2026-10-20T12:00:00Z.
const NOW: i64 = 1_792_497_600;

const HEADER: &str = "Notes:\n\"When exporting your connection data, you may notice that some of the email addresses are missing.\"\n\nFirst Name,Last Name,URL,Email Address,Company,Position,Connected On\n";

fn export(dir: &Path, name: &str, rows: &[&str]) -> PathBuf {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, format!("{HEADER}{}\n", rows.join("\n"))).unwrap();
    p
}

fn graph(dir: &Path) -> Graph {
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    let mut g =
        Graph::open_in_memory(Config::default(), providers, Box::new(FixedClock(NOW))).unwrap();
    let old = export(
        dir,
        "2025/Connections.csv",
        &[
            "Ana,Lee,https://www.linkedin.com/in/ana-lee-x1,,Stripe,Engineer,02 Feb 2023",
            "Tom,Okafor,https://www.linkedin.com/in/tom-okafor-x2,,Stripe,Engineering Manager,03 Mar 2023",
            "Kai,Wu,https://www.linkedin.com/in/kai-wu-x3,,Ramp,Analyst,04 Apr 2023",
            "Gone,Away,https://www.linkedin.com/in/gone-away-x4,,Fabrikam,Designer,05 May 2023",
        ],
    );
    g.import(&[old], &ImportRequest::default()).unwrap();
    let new = export(
        dir,
        "2026/Connections.csv",
        &[
            "Ana,Lee,https://www.linkedin.com/in/ana-lee-x1,,Anthropic,Engineer,02 Feb 2023",
            "Tom,Okafor,https://www.linkedin.com/in/tom-okafor-x2,,Stripe,Senior Engineering Manager,03 Mar 2023",
            "Kai,Wu,https://www.linkedin.com/in/kai-wu-x3,,Ramp,Analyst,04 Apr 2023",
            "New,Face,https://www.linkedin.com/in/new-face-x5,,Northwind,Founder,06 Jun 2026",
        ],
    );
    g.import(&[new], &ImportRequest::default()).unwrap();
    g
}

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| p["display_name"].as_str().unwrap().to_string())
        .collect()
}

fn run(g: &Graph, name: &str, input: Value) -> openstance_core::playbook::Outcome {
    g.run_playbook(name, &input, &RunOptions::default())
        .unwrap()
}

fn log_rows(g: &Graph) -> i64 {
    g.conn()
        .query_row("SELECT count(*) FROM enrich_log", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn accounts_reads_a_job_change_between_exports_as_alumni() {
    let dir = tempfile::tempdir().unwrap();
    let g = graph(dir.path());
    let out = run(
        &g,
        "accounts",
        json!({"companies": ["Stripe", "Anthropic"]}),
    );
    assert_eq!(out.status, RunStatus::Done);
    let rows = &out.result.as_ref().unwrap()["companies"];
    assert_eq!(names(&rows[0]["current"]), ["Tom Okafor"]);
    assert_eq!(names(&rows[0]["alumni"]), ["Ana Lee"]);
    assert_eq!(rows[0]["alumni"][0]["seen_in"], "linkedin_csv");
    assert!(rows[0]["alumni"][0]["left_seen"].is_string());
    assert_eq!(names(&rows[1]["current"]), ["Ana Lee"]);
    assert_eq!(names(&rows[1]["alumni"]), Vec::<String>::new());
    assert_eq!(log_rows(&g), 0);
}

#[test]
fn watch_diffs_the_latest_two_exports_with_zero_egress() {
    let dir = tempfile::tempdir().unwrap();
    let g = graph(dir.path());
    let out = run(&g, "watch", json!({"targets": ["Anthropic"]}));
    assert_eq!(out.status, RunStatus::Done);
    let r = out.result.as_ref().unwrap();
    let moves = r["moves"].as_array().unwrap();
    assert_eq!(moves.len(), 1, "{r}");
    assert_eq!(moves[0]["display_name"], "Ana Lee");
    assert_eq!(moves[0]["from"], "Stripe");
    assert_eq!(moves[0]["to"], "Anthropic");
    assert_eq!(moves[0]["detected_by"], "linkedin_diff");
    assert_eq!(moves[0]["target_hit"], true);
    assert_eq!(moves[0]["evidence"].as_array().unwrap().len(), 2);
    let titles = r["title_changes"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["display_name"], "Tom Okafor");
    assert_eq!(titles[0]["from"], "Engineering Manager");
    assert_eq!(titles[0]["to"], "Senior Engineering Manager");
    assert_eq!(names(&r["new_connections"]), ["New Face"]);
    assert_eq!(names(&r["removed_connections"]), ["Gone Away"]);
    assert_eq!(r["checked"], 0);
    assert!(r["sources"]["previous"]["id"].is_i64());
    assert!(r["sources"]["latest"]["id"].is_i64());
    assert_eq!(out.receipt.requests_sent, 0);
    assert_eq!(log_rows(&g), 0);
}
