//! `get_person` / `openstance who` (P2-CORE-04): the full `PersonDetail` of
//! one person reference (id, former anchor id, email, LinkedIn, @handle or
//! name).
//!
//! The record carries every displayed field with its provenance
//! (`field_sources`: `override` or `<source.kind>:<source.label>`), every
//! identity with its confidence and rule, all five score components with the
//! weights, preset and raw counts used, the most recent interactions and the
//! strongest mutuals by tie weight. `do_not_share` only affects sharing: the
//! person is returned locally and the flag is shown.

use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::json::PersonDetail;
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::search::person::{detail, DetailOptions};

/// Upper bound of `interactions` and `mutuals` (the MCP `get_person` schema).
pub const MAX_ITEMS: usize = 50;

/// Resolve `person` and build its `PersonDetail`. `opts` are clamped to
/// [`MAX_ITEMS`]. An id that was merged away resolves to the surviving person
/// and sets `resolved_from`.
pub fn get_person(
    conn: &Connection,
    cfg: &Config,
    person: &PersonKey,
    opts: DetailOptions,
    now: i64,
) -> Result<PersonDetail> {
    let r = resolve_person(conn, person)?;
    let opts = DetailOptions {
        interactions: opts.interactions.min(MAX_ITEMS),
        mutuals: opts.mutuals.min(MAX_ITEMS),
    };
    detail(conn, cfg, r.person, r.resolved_from, opts, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testkit::{indexed, NOW};
    use openstance_import::HandleKind;

    fn get(c: &Connection, key: PersonKey, opts: DetailOptions) -> PersonDetail {
        get_person(c, &Config::default(), &key, opts, NOW).unwrap()
    }

    #[test]
    fn resolves_by_id_email_and_name_to_the_same_person() {
        let c = indexed();
        let by_id = get(&c, PersonKey::Id(2), DetailOptions::default());
        let by_email = get(
            &c,
            PersonKey::Handle(HandleKind::Email, "Priya@Northwind.example".into()),
            DetailOptions::default(),
        );
        let by_name = get(
            &c,
            PersonKey::Name("priya raman".into()),
            DetailOptions::default(),
        );
        assert_eq!(by_id.person.id, 2);
        assert_eq!(by_id, by_email);
        assert_eq!(by_id, by_name);
        assert_eq!(by_id.resolved_from, None);
    }

    #[test]
    fn a_secondary_identity_id_resolves_with_resolved_from() {
        let c = indexed();
        let d = get(&c, PersonKey::Id(8), DetailOptions::default());
        assert_eq!(d.person.id, 2);
        assert_eq!(d.resolved_from, Some(8));
    }

    #[test]
    fn components_weights_and_raw_counts_explain_the_score() {
        let c = indexed();
        let d = get(&c, PersonKey::Id(2), DetailOptions::default());
        let k = &d.person.components;
        let w = &k.weights;
        let recombined = w.recency * k.recency
            + w.frequency * k.frequency
            + w.reciprocity * k.reciprocity
            + w.meetings * k.meetings
            + w.breadth * k.breadth;
        // The seeded components are not a real scoring run; what matters is
        // that every input of the sum is present, plus the raw counts.
        assert!(recombined > 0.0);
        assert_eq!(k.preset, "default");
        assert_eq!(k.half_life_days, 180.0);
        let raw = k.raw.as_ref().expect("who fills raw counts");
        assert!(raw.n_interactions >= 2, "{raw:?}");
        assert!(raw.n_meetings >= 1, "{raw:?}");
    }

    #[test]
    fn identities_carry_confidence_and_rule() {
        let c = indexed();
        let d = get(&c, PersonKey::Id(5), DetailOptions::default());
        let alias = d
            .identities
            .iter()
            .find(|i| i.value == "sam.ito@fabrikam.example")
            .unwrap();
        assert_eq!(alias.confidence, Some(0.98));
        assert_eq!(alias.match_rule.as_deref(), Some("gmail_alias"));
    }

    #[test]
    fn field_provenance_names_overrides_and_derived_sources() {
        let c = indexed();
        c.execute(
            "INSERT INTO override(person_id, field, value, set_at) VALUES (2, 'title', 'CEO', 9)",
            [],
        )
        .unwrap();
        c.execute("UPDATE person SET title = 'CEO' WHERE id = 2", [])
            .unwrap();
        c.execute(
            "UPDATE interaction_person SET company_hint = 'Northwind'
             WHERE interaction_id = 101 AND identity_id = 2",
            [],
        )
        .unwrap();
        let d = get(&c, PersonKey::Id(2), DetailOptions::default());
        assert_eq!(d.person.field_sources["title"].as_deref(), Some("override"));
        assert_eq!(
            d.person.field_sources["company"].as_deref(),
            Some("mbox:work gmail")
        );
        assert_eq!(d.overrides[0].field, "title");
    }

    #[test]
    fn do_not_share_people_are_returned_locally_with_the_flag() {
        let c = indexed();
        c.execute("UPDATE person SET do_not_share = 1 WHERE id = 3", [])
            .unwrap();
        let d = get(&c, PersonKey::Id(3), DetailOptions::default());
        assert!(d.person.do_not_share);
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["person"]["do_not_share"], true);
    }

    #[test]
    fn interactions_and_mutuals_are_limited_and_clamped() {
        let c = indexed();
        c.execute_batch(
            "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
               (2, 3, 'email', 0.3, 0, 1), (3, 7, 'meeting', 0.5, 0, 1);",
        )
        .unwrap();
        let d = get(
            &c,
            PersonKey::Id(3),
            DetailOptions {
                interactions: 1,
                mutuals: 1,
            },
        );
        assert_eq!(d.recent_interactions.len(), 1);
        assert_eq!(d.mutuals.len(), 1);
        assert_eq!(d.mutuals[0].person.id, 7, "strongest tie first");
        // Huge limits are clamped rather than rejected.
        let d = get(
            &c,
            PersonKey::Id(3),
            DetailOptions {
                interactions: usize::MAX,
                mutuals: usize::MAX,
            },
        );
        assert_eq!(d.mutuals.len(), 2);
        assert_eq!(d.recent_interactions.len(), 3);
    }

    #[test]
    fn unknown_and_ambiguous_references_are_errors() {
        let c = indexed();
        let e = get_person(
            &c,
            &Config::default(),
            &PersonKey::Name("Nobody Here".into()),
            DetailOptions::default(),
            NOW,
        )
        .unwrap_err();
        assert!(matches!(e, crate::Error::NotFound(_)));
        c.execute(
            "INSERT INTO person(id, display_name) VALUES (50, 'Priya Raman')",
            [],
        )
        .unwrap();
        let e = get_person(
            &c,
            &Config::default(),
            &PersonKey::Name("Priya Raman".into()),
            DetailOptions::default(),
            NOW,
        )
        .unwrap_err();
        assert!(matches!(e, crate::Error::Ambiguous { .. }));
    }
}
