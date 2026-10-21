use std::collections::HashSet;

use openstance_model::embed::{HashEmbedder, UnavailableEmbedder};
use openstance_model::{EmbedKind, Embedder, NullEmbedder};
use rusqlite::Connection;

use super::*;
use crate::search::testkit::graph;

fn idx(c: &mut Connection, e: &dyn Embedder) -> EmbedReport {
    index(c, e, &IndexOptions::default(), &mut |_| {}).unwrap()
}

fn vec_ids(c: &Connection) -> Vec<i64> {
    c.prepare("SELECT person_id FROM person_vec ORDER BY person_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn meta(c: &Connection, k: &str) -> Option<String> {
    c.query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| r.get(0))
        .ok()
}

#[test]
fn embed_text_follows_the_documented_template() {
    let c = graph();
    let t = embed_text(&c, 2).unwrap().unwrap();
    assert!(
        t.starts_with("Priya Raman. Co-founder & CEO at Northwind. San Francisco, CA\nTags"),
        "{t}"
    );
    assert!(t.contains("\nTags: investor-intro"), "{t}");
    assert!(t.contains("\nTopics: "), "{t}");
    assert!(t.contains("seed round notes"), "Re: prefix stripped: {t}");
    assert!(!t.contains("Re: seed"), "{t}");
    let lena = embed_text(&c, 6).unwrap().unwrap();
    assert_eq!(
        lena,
        "Lena Park. Partner at Halcyon Ventures. London. Angel investor in fintech\nTags: angels"
    );
    // empty parts are omitted, duplicate subjects collapse
    assert_eq!(
        embed_text(&c, 7).unwrap().unwrap(),
        "Ravi Iyer\nTopics: devtools dinner"
    );
}

#[test]
fn embed_text_never_includes_notes_and_skips_self_and_bulk() {
    let c = graph();
    let t = embed_text(&c, 3).unwrap().unwrap();
    assert!(!t.contains("secret-note-text"), "{t}");
    assert_eq!(embed_text(&c, 1).unwrap(), None, "self");
    assert_eq!(embed_text(&c, 4).unwrap(), None, "bulk");
    assert_eq!(embed_text(&c, 999).unwrap(), None, "missing");
}

#[test]
fn topics_are_normalized_ranked_and_capped() {
    let subjects = vec![
        ("Re: Fwd: Seed round".to_string(), 10),
        ("seed round".to_string(), 5),
        ("RE: seed round".to_string(), 1),
        ("Hi".to_string(), 50),
        ("Invitation: Coffee @ Tue".to_string(), 40),
        ("Accepted: Coffee".to_string(), 40),
        ("Board deck".to_string(), 30),
        ("Board deck".to_string(), 3),
        ("Pricing".to_string(), 60),
    ];
    let t = topics(&subjects, 20);
    assert_eq!(t, ["Seed round", "Board deck", "Pricing"]);
    let many: Vec<(String, i64)> = (0..50).map(|i| (format!("topic {i}"), i)).collect();
    assert_eq!(topics(&many, 20).len(), 20);
    assert_eq!(topics(&many, 20)[0], "topic 49", "ties break by recency");
}

#[test]
fn long_texts_are_capped() {
    let c = graph();
    c.execute(
        "UPDATE person SET bio = ?1 WHERE id = 3",
        [("word ".repeat(2000)).as_str()],
    )
    .unwrap();
    let t = embed_text(&c, 3).unwrap().unwrap();
    assert!(t.chars().count() <= MAX_TEXT_CHARS, "{}", t.len());
}

#[test]
fn text_hash_depends_on_embedder_and_text() {
    assert_ne!(text_hash("hash:384", "a"), text_hash("hash:64", "a"));
    assert_ne!(text_hash("hash:384", "a"), text_hash("hash:384", "b"));
    assert_eq!(text_hash("hash:384", "a"), text_hash("hash:384", "a"));
}

#[test]
fn first_run_embeds_every_eligible_person_and_a_second_run_none() {
    let mut c = graph();
    let e = HashEmbedder::new(384);
    assert_eq!(status(&c, &e).unwrap(), Status::NotIndexed);
    let r = idx(&mut c, &e);
    assert_eq!((r.embedded, r.unchanged, r.removed), (5, 0, 0));
    assert!(r.error.is_none() && r.skipped.is_none());
    assert_eq!(vec_ids(&c), [2, 3, 5, 6, 7]);
    assert_eq!(status(&c, &e).unwrap(), Status::Ready);
    assert_eq!(meta(&c, "embedder_id").as_deref(), Some("hash:384"));
    assert_eq!(meta(&c, "embedder_dims").as_deref(), Some("384"));
    assert_eq!(meta(&c, "embedder_state").as_deref(), Some("ready"));
    let again = idx(&mut c, &e);
    assert_eq!((again.embedded, again.unchanged), (0, 5));
}

#[test]
fn editing_one_title_re_embeds_exactly_one_person() {
    let mut c = graph();
    let e = HashEmbedder::new(384);
    idx(&mut c, &e);
    c.execute("UPDATE person SET title = 'VP Sales' WHERE id = 3", [])
        .unwrap();
    let r = idx(&mut c, &e);
    assert_eq!((r.embedded, r.unchanged), (1, 4));
    // a scoped pass over the edited person only
    c.execute("UPDATE person SET title = 'CTO' WHERE id = 3", [])
        .unwrap();
    let r = index(
        &mut c,
        &e,
        &IndexOptions {
            people: Some(vec![3]),
            ..IndexOptions::default()
        },
        &mut |_| {},
    )
    .unwrap();
    assert_eq!((r.embedded, r.unchanged), (1, 0));
}

#[test]
fn people_who_become_bulk_or_disappear_lose_their_vector() {
    let mut c = graph();
    let e = HashEmbedder::new(384);
    idx(&mut c, &e);
    c.execute("UPDATE person SET is_bulk = 1 WHERE id = 5", [])
        .unwrap();
    c.execute_batch(
        "UPDATE identity SET person_id = 2 WHERE person_id = 7;
         UPDATE interaction_person SET person_id = 2 WHERE person_id = 7;
         DELETE FROM person WHERE id = 7;",
    )
    .unwrap();
    let r = idx(&mut c, &e);
    assert_eq!(r.removed, 2);
    assert_eq!(vec_ids(&c), [2, 3, 6]);
    let n: i64 = c
        .query_row("SELECT count(*) FROM person_embed", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 3);
}

#[test]
fn a_changed_embedder_is_detected_and_only_force_reindexes() {
    let mut c = graph();
    idx(&mut c, &HashEmbedder::new(384));
    let other = HashEmbedder::new(64);
    assert_eq!(
        status(&c, &other).unwrap(),
        Status::Changed {
            stored: "hash:384".into(),
            configured: "hash:64".into()
        }
    );
    let r = idx(&mut c, &other);
    assert_eq!(r.embedded, 0);
    assert_eq!(r.skipped.as_deref(), Some(CHANGED_WARNING));
    assert_eq!(meta(&c, "person_vec_dims").as_deref(), Some("384"));
    assert_eq!(meta(&c, "embedder_id").as_deref(), Some("hash:384"));

    let r = index(
        &mut c,
        &other,
        &IndexOptions {
            force: true,
            ..IndexOptions::default()
        },
        &mut |_| {},
    )
    .unwrap();
    assert!(r.recreated);
    assert_eq!(r.embedded, 5);
    assert_eq!(meta(&c, "person_vec_dims").as_deref(), Some("64"));
    assert_eq!(status(&c, &other).unwrap(), Status::Ready);
    let sql: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'person_vec'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("float[64]"), "{sql}");
}

#[test]
fn force_with_the_same_embedder_re_embeds_everyone_without_recreating() {
    let mut c = graph();
    let e = HashEmbedder::new(384);
    idx(&mut c, &e);
    let r = index(
        &mut c,
        &e,
        &IndexOptions {
            force: true,
            ..IndexOptions::default()
        },
        &mut |_| {},
    )
    .unwrap();
    assert_eq!((r.embedded, r.recreated), (5, false));
}

#[test]
fn null_embedder_leaves_the_table_alone() {
    let mut c = graph();
    idx(&mut c, &HashEmbedder::new(384));
    assert_eq!(status(&c, &NullEmbedder).unwrap(), Status::Disabled);
    let r = idx(&mut c, &NullEmbedder);
    assert_eq!(r.embedded, 0);
    assert!(r.skipped.is_some());
    assert_eq!(vec_ids(&c).len(), 5);
}

#[test]
fn an_unavailable_model_is_reported_not_raised() {
    let mut c = graph();
    let e = UnavailableEmbedder {
        id: "fastembed:bge-small-en-v1.5".into(),
        dims: 384,
        reason: "not downloaded".into(),
    };
    let r = idx(&mut c, &e);
    assert_eq!(r.embedded, 0);
    assert!(r.error.as_deref().unwrap().contains("not downloaded"));
    assert!(vec_ids(&c).is_empty());
    assert_eq!(status(&c, &e).unwrap(), Status::Building);
    // once the model works, the same id resumes
    let ok = Renamed(HashEmbedder::new(384), "fastembed:bge-small-en-v1.5");
    let r = idx(&mut c, &ok);
    assert_eq!(r.embedded, 5);
    assert_eq!(status(&c, &ok).unwrap(), Status::Ready);
}

/// A hash embedder under another id.
struct Renamed(HashEmbedder, &'static str);

impl Embedder for Renamed {
    fn id(&self) -> &str {
        self.1
    }
    fn dims(&self) -> usize {
        self.0.dims()
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn embed(&self, t: &[&str], k: EmbedKind) -> openstance_model::Result<Vec<Vec<f32>>> {
        self.0.embed(t, k)
    }
}

#[test]
fn small_chunks_commit_separately_and_report_progress() {
    let mut c = graph();
    let mut seen = Vec::new();
    let r = index(
        &mut c,
        &HashEmbedder::new(384),
        &IndexOptions {
            chunk: 2,
            ..IndexOptions::default()
        },
        &mut |p| seen.push((p.done, p.total)),
    )
    .unwrap();
    assert_eq!(r.embedded, 5);
    assert_eq!(seen, [(0, 5), (2, 5), (4, 5), (5, 5)]);
}

#[test]
fn vector_candidates_rank_by_cosine_and_respect_the_allowed_set() {
    let mut c = graph();
    let e = HashEmbedder::new(384);
    idx(&mut c, &e);
    let q = e.vector("devtools dinner");
    let hits = vector_candidates(&c, &q, None, 200).unwrap();
    assert_eq!(hits[0].0, 7, "{hits:?}");
    assert!(hits[0].1 > hits[1].1);
    assert!(hits.iter().all(|h| (-1.0..=1.0001).contains(&h.1)));
    assert_eq!(hits.len(), 5);
    let top2 = vector_candidates(&c, &q, None, 2).unwrap();
    assert_eq!(top2.len(), 2);
    let allowed: HashSet<i64> = [2, 3].into_iter().collect();
    let r = vector_candidates(&c, &q, Some(&allowed), 200).unwrap();
    let mut ids: Vec<i64> = r.iter().map(|h| h.0).collect();
    ids.sort_unstable();
    assert_eq!(ids, [2, 3]);
    assert!(vector_candidates(&c, &q, Some(&HashSet::new()), 200)
        .unwrap()
        .is_empty());
    // a query vector of the wrong length is not an error, just no hits
    assert!(vector_candidates(&c, &[1.0; 8], None, 200)
        .unwrap()
        .is_empty());
}
