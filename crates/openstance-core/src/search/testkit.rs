//! A small seeded graph for search unit tests (direct SQL inserts; every
//! name and domain is fictional). `NOW` is 2026-10-20 12:00 UTC.
//!
//! | id | person | notes |
//! |---|---|---|
//! | 1 | Owen Ryder | self |
//! | 2 | Priya Raman | Co-founder & CEO, Northwind, San Francisco; meetings; replied; tag investor-intro |
//! | 3 | Marcus Lee | Staff Engineer, Contoso, Berlin; never replied; one meeting |
//! | 4 | Digest Weekly | bulk |
//! | 5 | Sam Ito | Product Manager, Fabrikam; email at stripe.example; lost touch |
//! | 6 | Lena Park | Partner, Halcyon Ventures, London; tag angels; connection only |
//! | 7 | Ravi Iyer | no title/company; email at contoso.example; "devtools dinner" |
//!
//! Compiled for core's own tests and, with the `test-util` feature, for
//! integration tests in other crates (the CLI).

#![allow(clippy::type_complexity)]

use rusqlite::Connection;

/// Seconds per day.
pub const DAY: i64 = 86_400;

/// 2026-10-20T12:00:00Z.
pub const NOW: i64 = 1_792_454_400 + 12 * 3600;

/// A migrated in-memory database with the graph above.
pub fn graph() -> Connection {
    let mut c = crate::db::open_in_memory().expect("open");
    crate::db::migrate(&mut c, None, 0).expect("migrate");
    seed(&c);
    c
}

/// `NOW - days` days.
pub fn ago(days: i64) -> i64 {
    NOW - days * DAY
}

/// Insert the graph above into a migrated, empty database.
pub fn seed(c: &Connection) {
    c.execute_batch(
        "INSERT INTO source(id, kind, label, imported_at) VALUES
           (1, 'mbox', 'work gmail', 0),
           (2, 'linkedin_csv', 'linkedin 2026-10', 0),
           (3, 'ics', 'work calendar', 0);",
    )
    .expect("sources");
    let people: &[(
        i64,
        &str,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<&str>,
    )] = &[
        (1, "Owen Ryder", None, None, None, None),
        (
            2,
            "Priya Raman",
            Some("Co-founder & CEO"),
            Some("Northwind"),
            Some("San Francisco, CA"),
            None,
        ),
        (
            3,
            "Marcus Lee",
            Some("Staff Engineer"),
            Some("Contoso"),
            Some("Berlin"),
            None,
        ),
        (4, "Digest Weekly", None, None, None, None),
        (
            5,
            "Sam Ito",
            Some("Product Manager"),
            Some("Fabrikam"),
            None,
            None,
        ),
        (
            6,
            "Lena Park",
            Some("Partner"),
            Some("Halcyon Ventures"),
            Some("London"),
            Some("Angel investor in fintech"),
        ),
        (7, "Ravi Iyer", None, None, None, None),
    ];
    // (id, last_contact, score, r, f, c, m, b, is_bulk, is_self)
    let scores: &[(i64, Option<i64>, f64, f64, f64, f64, f64, f64, i64, i64)] = &[
        (1, None, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0, 1),
        (2, Some(ago(5)), 0.8, 0.95, 0.9, 0.7, 0.6, 0.5, 0, 0),
        (3, Some(ago(20)), 0.5, 0.5, 0.6, 0.0, 0.3, 0.25, 0, 0),
        (4, Some(ago(2)), 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1, 0),
        (5, Some(ago(400)), 0.35, 0.1, 0.8, 0.5, 0.3, 0.5, 0, 0),
        (6, None, 0.2, 0.3, 0.1, 0.0, 0.0, 0.25, 0, 0),
        (7, Some(ago(19)), 0.45, 0.6, 0.4, 0.5, 0.3, 0.25, 0, 0),
    ];
    for (p, s) in people.iter().zip(scores) {
        c.execute(
            "INSERT INTO person(id, display_name, title, company, location, bio, last_contact,
                score, score_r, score_f, score_c, score_m, score_b, is_bulk, is_self)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                p.0, p.1, p.2, p.3, p.4, p.5, s.1, s.2, s.3, s.4, s.5, s.6, s.7, s.8, s.9
            ],
        )
        .expect("person");
    }
    c.execute_batch(
        "INSERT INTO identity(id, kind, value, source_id, person_id, confidence, match_rule) VALUES
           (1, 'email', 'owner@example.com', 1, 1, 1.0, 'anchor'),
           (2, 'email', 'priya@northwind.example', 1, 2, 1.0, 'anchor'),
           (3, 'email', 'marcus@contoso.example', 1, 3, 1.0, 'anchor'),
           (4, 'email', 'news@digest.example', 1, 4, 1.0, 'anchor'),
           (5, 'email', 'sam@stripe.example', 1, 5, 1.0, 'anchor'),
           (6, 'linkedin_url', 'lena-park', 2, 6, 1.0, 'anchor'),
           (7, 'email', 'ravi@contoso.example', 1, 7, 1.0, 'anchor'),
           (8, 'linkedin_url', 'priya-raman-nw', 2, 2, 1.0, 'exact'),
           (9, 'email', 'sam.ito@fabrikam.example', 1, 5, 0.98, 'gmail_alias');
         INSERT INTO override(person_id, field, value, set_at) VALUES
           (6, 'tag', 'angels', 1),
           (8, 'tag', 'investor-intro', 2),
           (3, 'note', 'secret-note-text', 3);",
    )
    .expect("identities");
    // (id, source, kind, at, subject, direction, participants [(identity, role)])
    type Ix<'a> = (
        i64,
        i64,
        &'a str,
        i64,
        Option<&'a str>,
        &'a str,
        &'a [(i64, &'a str)],
    );
    let ixs: &[Ix] = &[
        (
            101,
            1,
            "email",
            ago(5),
            Some("Re: seed round notes"),
            "in",
            &[(2, "from"), (1, "to")],
        ),
        (
            102,
            3,
            "meeting",
            ago(30),
            Some("Northwind x Owen coffee"),
            "mutual",
            &[(2, "attendee"), (1, "organizer")],
        ),
        (
            103,
            1,
            "email",
            ago(60),
            Some("Board deck draft"),
            "out",
            &[(1, "from"), (2, "to"), (3, "cc")],
        ),
        (
            104,
            1,
            "email",
            ago(2),
            Some("Weekly digest #12"),
            "in",
            &[(4, "from")],
        ),
        (
            105,
            1,
            "email",
            ago(400),
            Some("Q3 hiring plan"),
            "in",
            &[(5, "from")],
        ),
        (
            106,
            2,
            "connection",
            ago(200),
            None,
            "none",
            &[(6, "subject")],
        ),
        (
            107,
            3,
            "meeting",
            ago(20),
            Some("devtools dinner"),
            "mutual",
            &[(7, "attendee"), (3, "attendee")],
        ),
        (
            108,
            1,
            "email",
            ago(19),
            Some("Re: devtools dinner"),
            "in",
            &[(7, "from"), (1, "to")],
        ),
        (
            109,
            1,
            "email",
            ago(800),
            Some("Old thread"),
            "in",
            &[(3, "from")],
        ),
        (
            110,
            3,
            "meeting",
            NOW + 5 * DAY,
            Some("Northwind offsite"),
            "mutual",
            &[(8, "attendee")],
        ),
    ];
    for (id, src, kind, at, subject, dir, parts) in ixs {
        c.execute(
            "INSERT INTO interaction(id, source_id, kind, at, subject, external_id, direction)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, src, kind, at, subject, format!("x{id}"), dir],
        )
        .expect("interaction");
        for (ident, role) in *parts {
            c.execute(
                "INSERT INTO interaction_person(interaction_id, identity_id, role, person_id)
                 VALUES (?1, ?2, ?3, (SELECT person_id FROM identity WHERE id = ?2))",
                rusqlite::params![id, ident, role],
            )
            .expect("participant");
        }
    }
}

/// [`graph`] with `person_fts` populated.
pub fn indexed() -> Connection {
    let c = graph();
    super::fts::reindex_all(&c).expect("reindex");
    c
}
