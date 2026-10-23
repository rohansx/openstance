//! Self detection when `[me] identities` is empty (plain `openstance init`,
//! the UI/e2e setup). An import whose self candidate is unambiguous (the
//! `From` of at least 80% of the mbox's `Sent` mail, the X archive's own
//! account, the only LinkedIn owner seen in SENT messages and invitations)
//! makes that handle the self person: it is reported in the import
//! response, stored in the graph, and the owner stops showing up as a
//! contact, a mutual or a path hop. `[me] identities` in config always
//! wins.

use std::path::{Path, PathBuf};

use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::{Config, FixedClock, Graph, Providers};

const NOW: i64 = 1_791_000_000; // 2026-10-03

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn graph(cfg: Config) -> Graph {
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    Graph::open_in_memory(cfg, providers, Box::new(FixedClock(NOW))).unwrap()
}

/// One message per `(label, from, to, subject)`; replies share a thread.
fn mbox(dir: &Path, msgs: &[(&str, &str, &str, &str)]) -> PathBuf {
    let mut s = String::new();
    for (i, (label, from, to, subject)) in msgs.iter().enumerate() {
        s.push_str(&format!(
            "From x@example.com Fri Oct 0{d} 09:00:00 2026\n\
             X-GM-THRID: {t}\n\
             X-Gmail-Labels: {label}\n\
             Message-ID: <m{i}@example.com>\n\
             Date: Fri, 0{d} Oct 2026 09:00:00 +0000\n\
             From: {from}\n\
             To: {to}\n\
             Subject: {subject}\n\n(body)\n\n",
            d = 1 + i % 9,
            t = 900 + i / 2,
        ));
    }
    let p = dir.join("takeout.mbox");
    std::fs::write(&p, s).unwrap();
    p
}

const ME: &str = "Jordan Avery <you@example.com>";
const MAYA: &str = "Maya Chen <maya@ledgerline.example>";
const SAM: &str = "Sam Ortiz <sam@corvid.example>";

fn owner_mbox(dir: &Path) -> PathBuf {
    mbox(
        dir,
        &[
            ("Sent", ME, MAYA, "Breakfast"),
            ("Inbox", MAYA, ME, "Re: Breakfast"),
            ("Sent", ME, SAM, "Renewal"),
            ("Inbox", SAM, ME, "Re: Renewal"),
            (
                "Sent",
                ME,
                &format!("{MAYA}, {SAM}"),
                "Intro: Maya, meet Sam",
            ),
            (
                "Inbox",
                MAYA,
                &format!("{ME}, {SAM}"),
                "Re: Intro: Maya, meet Sam",
            ),
        ],
    )
}

fn is_self(g: &Graph, kind: &str, value: &str) -> Option<bool> {
    g.conn()
        .query_row(
            "SELECT p.is_self FROM identity i JOIN person p ON p.id = i.person_id
             WHERE i.kind = ?1 AND i.value = ?2",
            [kind, value],
            |r| r.get(0),
        )
        .ok()
}

fn names(g: &Graph) -> Vec<String> {
    let mut st = g
        .conn()
        .prepare("SELECT display_name FROM person WHERE is_self = 0 ORDER BY display_name")
        .unwrap();
    st.query_map([], |r| r.get::<_, Option<String>>(0))
        .unwrap()
        .map(|r| r.unwrap().unwrap_or_default())
        .collect()
}

#[test]
fn the_dominant_sent_from_becomes_the_self_person() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = graph(Config::default());
    let resp = g
        .import(&[owner_mbox(dir.path())], &ImportRequest::default())
        .unwrap();
    assert_eq!(resp.sources[0].self_adopted, ["email:you@example.com"]);
    assert_eq!(is_self(&g, "email", "you@example.com"), Some(true));
    assert_eq!(
        names(&g),
        ["Maya Chen", "Sam Ortiz"],
        "the owner is no contact"
    );
    assert_eq!(g.counts().unwrap().persons_excluding_self, 2);

    // Not a mutual or a hop: Maya and Sam share the intro thread, so the
    // only path to Maya goes through Sam.
    let maya = openstance_core::resolve::lookup::PersonKey::parse("maya@ledgerline.example");
    let how = g.how_connected(&maya, Default::default()).unwrap();
    let v = serde_json::to_value(&how).unwrap().to_string();
    assert!(!v.contains("Jordan Avery"), "{v}");

    // Stored in the graph: a full rebuild keeps it.
    g.rebuild().unwrap();
    assert_eq!(is_self(&g, "email", "you@example.com"), Some(true));
}

#[test]
fn an_ambiguous_sent_from_is_only_proposed() {
    let dir = tempfile::tempdir().unwrap();
    let other = "Desk <desk@example.com>";
    let p = mbox(
        dir.path(),
        &[
            ("Sent", ME, MAYA, "a"),
            ("Sent", other, MAYA, "b"),
            ("Sent", ME, SAM, "c"),
            ("Sent", other, SAM, "d"),
        ],
    );
    let mut g = graph(Config::default());
    let resp = g.import(&[p], &ImportRequest::default()).unwrap();
    assert!(resp.sources[0].self_adopted.is_empty());
    assert_eq!(resp.sources[0].stats.self_candidates.len(), 1);
    assert_eq!(is_self(&g, "email", "you@example.com"), Some(false));
}

#[test]
fn me_identities_in_config_win() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.me.identities = vec!["email:desk@example.com".into()];
    let mut g = graph(cfg);
    let resp = g
        .import(&[owner_mbox(dir.path())], &ImportRequest::default())
        .unwrap();
    assert!(resp.sources[0].self_adopted.is_empty());
    assert_eq!(is_self(&g, "email", "you@example.com"), Some(false));
}

#[test]
fn x_archive_account_is_adopted_and_later_imports_add_handles() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = graph(Config::default());
    let resp = g
        .import(&[owner_mbox(dir.path())], &ImportRequest::default())
        .unwrap();
    assert_eq!(resp.sources[0].self_adopted, ["email:you@example.com"]);
    let resp = g
        .import(
            &[fixtures().join("x/twitter-archive.zip")],
            &ImportRequest::default(),
        )
        .unwrap();
    let adopted = &resp.sources[0].self_adopted;
    assert_eq!(adopted.len(), 2, "{adopted:?}");
    assert!(adopted.iter().any(|h| h.starts_with("x_user_id:")));
    assert!(adopted.iter().any(|h| h.starts_with("x_handle:")));
    // One self person holds the mailbox and the X account.
    let selves: i64 = g
        .conn()
        .query_row("SELECT count(*) FROM person WHERE is_self = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(selves, 1);
    let x_self = adopted
        .iter()
        .find(|h| h.starts_with("x_user_id:"))
        .unwrap()
        .split_once(':')
        .unwrap();
    assert_eq!(is_self(&g, x_self.0, x_self.1), Some(true));
}

#[test]
fn linkedin_owner_from_sent_messages_is_adopted() {
    let mut g = graph(Config::default());
    let resp = g
        .import(
            &[fixtures().join("linkedin/messages.csv")],
            &ImportRequest::default(),
        )
        .unwrap();
    assert_eq!(
        resp.sources[0].self_adopted,
        ["linkedin_url:owen-ryder-example"]
    );
    assert_eq!(
        is_self(&g, "linkedin_url", "owen-ryder-example"),
        Some(true)
    );
}
