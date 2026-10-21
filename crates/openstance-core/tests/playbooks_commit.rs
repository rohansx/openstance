//! P4-CORE-04 / P4-CORE-05 on a real import (so resolution can replay what
//! the playbooks commit): `refresh` writes profiles under an
//! `enrich:<backend>` source without touching overrides, and `verify`
//! reuses a fresh verdict instead of paying again. All data is fictional.

use std::path::Path;

use openstance_core::enrich::mock::MockEnricher;
use openstance_core::enrich::{EnrichData, EnrichRequest, ProfileData, VerifyStatus};
use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::overrides::{Edit, OverrideField};
use openstance_core::playbook::{RunOptions, RunStatus, StepDecision};
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::{Config, FixedClock, Graph, PersonId, Providers};
use serde_json::json;

/// 2026-10-20T12:00:00Z.
const NOW: i64 = 1_792_497_600;

/// `n` threads: I write, they answer.
fn mbox(dir: &Path, people: &[(&str, &str, usize)]) -> std::path::PathBuf {
    let mut s = String::new();
    let mut k = 0;
    for (name, email, n) in people {
        for i in 0..*n {
            k += 1;
            let day = 1 + (k % 27);
            s.push_str(&format!(
                "From you@example.com Thu Oct {day:02} 09:00:00 2026\nX-GM-THRID: {k}\nX-Gmail-Labels: Sent\nMessage-ID: <o{k}@example.com>\nDate: Thu, {day:02} Oct 2026 09:00:00 +0000\nFrom: Jordan Avery <you@example.com>\nTo: {name} <{email}>\nSubject: Topic {i}\n\n(body)\n\n\
                 From {email} Thu Oct {day:02} 11:00:00 2026\nX-GM-THRID: {k}\nX-Gmail-Labels: Inbox\nMessage-ID: <i{k}@x.example>\nDate: Thu, {day:02} Oct 2026 11:00:00 +0000\nFrom: {name} <{email}>\nTo: Jordan Avery <you@example.com>\nSubject: Re: Topic {i}\nIn-Reply-To: <o{k}@example.com>\n\n(body)\n\n"
            ));
        }
    }
    let p = dir.join("mail.mbox");
    std::fs::write(&p, s).unwrap();
    p
}

fn graph(dir: &Path) -> Graph {
    let mut cfg = Config::default();
    cfg.me.identities = vec!["email:you@example.com".into()];
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    let mut g = Graph::open_in_memory(cfg, providers, Box::new(FixedClock(NOW))).unwrap();
    let file = mbox(
        dir,
        &[
            ("Stale Person", "stale@contoso.example", 4),
            ("Edited Person", "edited@contoso.example", 4),
        ],
    );
    g.import(&[file], &ImportRequest::default()).unwrap();
    g
}

fn id(g: &Graph, key: &str) -> PersonId {
    resolve_person(g.conn(), &PersonKey::parse(key))
        .unwrap()
        .person
}

fn field(g: &Graph, p: PersonId, f: &str) -> Option<String> {
    g.conn()
        .query_row(&format!("SELECT {f} FROM person WHERE id = ?1"), [p], |r| {
            r.get(0)
        })
        .unwrap()
}

fn opts(allow: bool) -> RunOptions {
    RunOptions {
        allow_egress: allow,
        ..RunOptions::default()
    }
}

#[test]
fn refresh_commits_profiles_under_an_enrich_source_and_leaves_overrides_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = graph(dir.path());
    let stale = id(&g, "stale@contoso.example");
    let edited = id(&g, "edited@contoso.example");
    g.edit(
        &PersonKey::Id(edited),
        &Edit {
            set: vec![(OverrideField::Title, String::new())],
            ..Edit::default()
        },
    )
    .unwrap();
    let overrides: i64 = g
        .conn()
        .query_row("SELECT count(*) FROM override", [], |r| r.get(0))
        .unwrap();
    let request = EnrichRequest::Profile {
        name: "Stale Person".into(),
        company: field(&g, stale, "company"),
        linkedin: None,
    };
    let m = MockEnricher::new().with_fetched_at(NOW).with_response(
        request.clone(),
        EnrichData::Profile(ProfileData {
            title: Some("Head of Data".into()),
            company: Some("Contoso".into()),
            ..ProfileData::default()
        }),
    );
    let out = g
        .run_playbook_with(&m, "refresh", &json!({}), &opts(true))
        .unwrap();
    assert_eq!(out.status, RunStatus::Done, "{:?}", out.notes);
    let sent: Vec<EnrichRequest> = m.calls().into_iter().map(|j| j.request).collect();
    assert_eq!(sent, [request], "the overridden person is not refreshed");
    let stale = id(&g, "stale@contoso.example");
    assert_eq!(field(&g, stale, "title").as_deref(), Some("Head of Data"));
    let kind: String = g
        .conn()
        .query_row(
            "SELECT s.kind FROM enrich_log e JOIN source s ON s.id = e.source_id
             WHERE e.op = 'profile' AND e.status = 'ok'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "enrich:mock");
    let after: i64 = g
        .conn()
        .query_row("SELECT count(*) FROM override", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, overrides);
    // Rebuild replays the enrichment.
    g.rebuild().unwrap();
    let stale = id(&g, "stale@contoso.example");
    assert_eq!(field(&g, stale, "title").as_deref(), Some("Head of Data"));
}

#[test]
fn verify_reuses_a_fresh_verification_instead_of_paying_again() {
    let dir = tempfile::tempdir().unwrap();
    let g = graph(dir.path());
    let m = MockEnricher::new().with_fetched_at(NOW).with_response(
        EnrichRequest::VerifyEmail {
            email: "stale@contoso.example".into(),
        },
        EnrichData::Verification {
            email: "stale@contoso.example".into(),
            status: VerifyStatus::Deliverable,
        },
    );
    let first = g
        .run_playbook_with(
            &m,
            "verify",
            &json!({"person": "Stale Person"}),
            &opts(true),
        )
        .unwrap();
    assert_eq!(first.result.as_ref().unwrap()["status"], "deliverable");
    let again = g
        .run_playbook_with(
            &m,
            "verify",
            &json!({"person": "Stale Person"}),
            &opts(true),
        )
        .unwrap();
    assert_eq!(m.calls().len(), 1, "the second run is within fresh_days");
    assert_eq!(again.status, RunStatus::Done);
    assert_eq!(again.result.as_ref().unwrap()["status"], "deliverable");
    assert_eq!(
        again.plan.steps[1].will_run,
        StepDecision::SkipLocalSufficient
    );
    assert_eq!(again.receipt.requests_sent, 0);
}
