//! The one aggregate loader behind scoring and bulk detection: a single pass
//! over `interaction_person` joined with `interaction` and `source`, grouped
//! per person in Rust (scoring.md section 7). Everything here is SQL plus
//! counting; the formulas live in [`super::components`].

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::{params, Connection, OptionalExtension};

use super::{Channel, ScoreInputs};
use crate::error::Result;
use crate::PersonId;

const DAY: i64 = 86_400;
/// A DM counts as a reply within this window after my last DM (2.4).
const DM_REPLY_WINDOW: i64 = 14 * DAY;
/// Subject-fallback threading window when no `thread_key` exists (2.4).
const SUBJECT_THREAD_WINDOW: i64 = 30 * DAY;

/// Everything scoring and bulk detection need about one person.
#[derive(Debug, Clone, Default)]
pub(crate) struct Agg {
    /// Raw score inputs (`days_since_last` filled for the loader's `now`).
    pub inputs: ScoreInputs,
    /// Earliest non-`contact` interaction.
    pub first_seen: Option<i64>,
    /// `person.is_self`.
    pub is_self: bool,
    /// email/dm interactions where they are `from`/`sender`.
    pub n_from: u32,
    /// email interactions where they are `from`/`sender`.
    pub n_from_email: u32,
    /// ... of which carried list headers.
    pub n_from_unsub: u32,
    /// Interactions of any kind with `direction = 'out'` addressed to them.
    pub n_out_any: u32,
    /// Every interaction they appear on (contact rows included).
    pub n_all: u32,
    /// Every interaction is list/auto mail where they are only the sender.
    pub only_auto: bool,
    /// `thread_key`s of the email/dm interactions they appear on.
    pub thread_keys: BTreeSet<String>,
    /// Their normalized email identities.
    pub emails: Vec<String>,
    /// Latest `is_bulk` override (`None` = no override or tombstone).
    pub bulk_override: Option<bool>,
}

/// `[1,2,3]` for `json_each`.
pub(crate) fn json_ids(ids: &[PersonId]) -> String {
    serde_json::to_string(ids).expect("i64 slice serializes")
}

/// Load aggregates for `people` (or every person). People without
/// interactions are present with zero counts; ids not in `person` are
/// absent.
pub(crate) fn load(
    conn: &Connection,
    people: Option<&[PersonId]>,
    now: i64,
) -> Result<BTreeMap<PersonId, Agg>> {
    let filter = people.map(json_ids);
    let mut out: BTreeMap<PersonId, Agg> = BTreeMap::new();

    // People.
    {
        let sql = match filter {
            Some(_) => {
                "SELECT id, is_self FROM person WHERE id IN (SELECT value FROM json_each(?1))"
            }
            None => "SELECT id, is_self FROM person WHERE ?1 IS NULL",
        };
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map(params![filter], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? != 0))
        })?;
        for row in rows {
            let (id, is_self) = row?;
            out.insert(
                id,
                Agg {
                    is_self,
                    only_auto: true,
                    ..Agg::default()
                },
            );
        }
    }
    if out.is_empty() {
        return Ok(out);
    }

    // Email identities.
    {
        let sql = match filter {
            Some(_) => {
                "SELECT person_id, value FROM identity
                 WHERE kind = 'email' AND person_id IN (SELECT value FROM json_each(?1))
                 ORDER BY person_id, id"
            }
            None => {
                "SELECT person_id, value FROM identity
                 WHERE kind = 'email' AND person_id IS NOT NULL AND ?1 IS NULL
                 ORDER BY person_id, id"
            }
        };
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map(params![filter], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (p, v) = row?;
            if let Some(a) = out.get_mut(&p) {
                a.emails.push(v);
            }
        }
    }

    // `is_bulk` overrides: keyed by anchor identity, so map through identity
    // to the current cluster. Last write (set_at, then id) wins; a NULL
    // value is a tombstone.
    {
        let sql = match filter {
            Some(_) => {
                "SELECT i.person_id, o.value FROM override o
                 JOIN identity i ON i.id = o.person_id
                 WHERE o.field = 'is_bulk' AND i.person_id IN (SELECT value FROM json_each(?1))
                 ORDER BY o.set_at, o.id"
            }
            None => {
                "SELECT i.person_id, o.value FROM override o
                 JOIN identity i ON i.id = o.person_id
                 WHERE o.field = 'is_bulk' AND i.person_id IS NOT NULL AND ?1 IS NULL
                 ORDER BY o.set_at, o.id"
            }
        };
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map(params![filter], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (p, v) = row?;
            if let Some(a) = out.get_mut(&p) {
                a.bulk_override = match v.as_deref().map(str::trim) {
                    Some("1") | Some("true") => Some(true),
                    Some("0") | Some("false") => Some(false),
                    _ => None,
                };
            }
        }
    }

    // Interactions, one row per (person, interaction), roles collapsed.
    let person_filter = match filter {
        Some(_) => "ip.person_id IN (SELECT value FROM json_each(?1))",
        None => "ip.person_id IS NOT NULL AND ?1 IS NULL",
    };
    let sql = format!(
        "SELECT ip.person_id, i.id, i.kind, i.at, i.direction, i.thread_key, i.subject,
                i.list_unsub, s.kind, group_concat(ip.role, ','),
                CASE WHEN i.direction = 'in' AND i.kind IN ('email','dm')
                          AND i.thread_key IS NOT NULL
                     THEN NOT EXISTS (
                          SELECT 1 FROM interaction j
                          WHERE j.thread_key = i.thread_key AND j.kind = i.kind
                            AND (j.at < i.at OR (j.at = i.at AND j.id < i.id)))
                END
         FROM interaction_person ip
         JOIN interaction i ON i.id = ip.interaction_id
         JOIN source s ON s.id = i.source_id
         WHERE {person_filter}
         GROUP BY ip.person_id, i.id
         ORDER BY ip.person_id, i.at, i.id"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params![filter], |r| {
        Ok(Row {
            person: r.get(0)?,
            kind: r.get(2)?,
            at: r.get(3)?,
            direction: r.get(4)?,
            thread_key: r.get(5)?,
            subject: r.get(6)?,
            list_unsub: r.get::<_, i64>(7)? != 0,
            source_kind: r.get(8)?,
            roles: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
            is_root: r.get::<_, Option<bool>>(10)?.unwrap_or(false),
        })
    })?;

    let mut current: Option<(PersonId, Threading)> = None;
    for row in rows {
        let row = row?;
        if current.as_ref().map(|(p, _)| *p) != Some(row.person) {
            if let Some((p, t)) = current.take() {
                finish(out.get_mut(&p), t, now);
            }
            current = Some((row.person, Threading::default()));
        }
        let (_, threading) = current.as_mut().expect("set above");
        if let Some(agg) = out.get_mut(&row.person) {
            accumulate(agg, threading, &row, now);
        }
    }
    if let Some((p, t)) = current.take() {
        finish(out.get_mut(&p), t, now);
    }
    // People with no interactions still need channels sorted (empty) and
    // `only_auto` false.
    for a in out.values_mut() {
        if a.n_all == 0 {
            a.only_auto = false;
        }
    }
    Ok(out)
}

/// Does any `out` interaction share a thread with these keys?
pub(crate) fn owner_wrote_on_threads(conn: &Connection, keys: &BTreeSet<String>) -> Result<bool> {
    if keys.is_empty() {
        return Ok(false);
    }
    let json = serde_json::to_string(keys)?;
    let hit: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM interaction
             WHERE direction = 'out' AND thread_key IN (SELECT value FROM json_each(?1))
             LIMIT 1",
            [json],
            |r| r.get(0),
        )
        .optional()?;
    Ok(hit.is_some())
}

struct Row {
    person: PersonId,
    kind: String,
    at: i64,
    direction: Option<String>,
    thread_key: Option<String>,
    subject: Option<String>,
    list_unsub: bool,
    source_kind: String,
    roles: String,
    is_root: bool,
}

/// Per-person threading state while rows stream by in time order.
#[derive(Default)]
struct Threading {
    /// Latest outbound email/dm to them per `(kind, thread_key)`.
    last_out: HashMap<(String, String), i64>,
    /// Subject fallback: `(kind, normalized subject)` -> times of any
    /// email/dm without a thread key, and of outbound ones to them.
    by_subject: HashMap<(String, String), (Vec<i64>, Vec<i64>)>,
    /// Channels seen.
    channels: BTreeSet<Channel>,
}

fn has_role(roles: &str, wanted: &[&str]) -> bool {
    roles.split(',').any(|r| wanted.contains(&r))
}

const FROM_ROLES: &[&str] = &["from", "sender"];
const TO_ROLES: &[&str] = &["to", "cc", "bcc", "recipient", "member", "attendee"];

fn accumulate(a: &mut Agg, t: &mut Threading, row: &Row, now: i64) {
    let from_like = has_role(&row.roles, FROM_ROLES);
    let to_like = has_role(&row.roles, TO_ROLES);
    let dir = row.direction.as_deref().unwrap_or("none");
    let kind = row.kind.as_str();

    // Bulk signals look at every row.
    a.n_all += 1;
    let only_sender = row.roles.split(',').all(|r| FROM_ROLES.contains(&r));
    if !(kind == "email" && only_sender && row.list_unsub) {
        a.only_auto = false;
    }
    if dir == "out" && to_like {
        a.n_out_any += 1;
    }
    if matches!(kind, "email" | "dm") {
        if let Some(k) = &row.thread_key {
            a.thread_keys.insert(k.clone());
        }
        if from_like {
            a.n_from += 1;
            if kind == "email" {
                a.n_from_email += 1;
                if row.list_unsub {
                    a.n_from_unsub += 1;
                }
            }
        }
    }

    // Scoring. `contact` rows never count; future meetings do not count yet.
    if kind == "contact" || (kind == "meeting" && row.at > now) {
        return;
    }
    a.first_seen = Some(a.first_seen.map_or(row.at, |f| f.min(row.at)));
    if let Some(c) = Channel::from_source_kind(&row.source_kind) {
        t.channels.insert(c);
    }
    let contact_group = matches!(kind, "email" | "meeting" | "dm" | "connection");
    if !contact_group {
        return; // follow / mention: breadth only
    }
    let i = &mut a.inputs;
    i.n_interactions += 1;
    i.last_contact = Some(i.last_contact.map_or(row.at, |l| l.max(row.at)));
    if kind == "meeting" {
        i.n_meetings += 1;
        return;
    }
    if kind == "connection" {
        return;
    }

    // email / dm: reciprocity.
    let outbound = dir == "out" && to_like;
    let inbound = dir == "in" && from_like;
    match &row.thread_key {
        Some(key) => {
            let k = (row.kind.clone(), key.clone());
            if inbound {
                if row.is_root {
                    i.n_threads_started += 1;
                } else if let Some(out_at) = t.last_out.get(&k) {
                    let in_window = kind != "dm" || row.at - out_at <= DM_REPLY_WINDOW;
                    if *out_at <= row.at && in_window {
                        i.n_replies += 1;
                    }
                }
            }
            if outbound {
                i.n_out += 1;
                t.last_out.insert(k, row.at);
            }
        }
        None => {
            let subj = normalize_subject(row.subject.as_deref().unwrap_or(""));
            let k = (row.kind.clone(), subj.clone());
            let entry = t.by_subject.entry(k).or_default();
            if inbound && !subj.is_empty() {
                let recent = |v: &Vec<i64>| {
                    v.iter()
                        .any(|at| *at <= row.at && row.at - at <= SUBJECT_THREAD_WINDOW)
                };
                if !recent(&entry.0) {
                    i.n_threads_started += 1;
                } else if recent(&entry.1) {
                    i.n_replies += 1;
                }
            }
            if outbound {
                i.n_out += 1;
                entry.1.push(row.at);
            }
            entry.0.push(row.at);
        }
    }
}

fn finish(a: Option<&mut Agg>, t: Threading, now: i64) {
    let Some(a) = a else { return };
    a.inputs.channels = t.channels.into_iter().collect();
    a.inputs.days_since_last = a
        .inputs
        .last_contact
        .map(|l| ((now - l) as f64 / DAY as f64).max(0.0));
}

/// Subject-fallback thread key: lowercase, strip `Re:` / `Fwd:` / `Fw:` /
/// `AW:` / `WG:` prefixes, collapse whitespace.
pub(crate) fn normalize_subject(s: &str) -> String {
    let mut cur = s.trim().to_lowercase();
    loop {
        let before = cur.len();
        for p in ["re:", "fwd:", "fw:", "aw:", "wg:"] {
            if let Some(rest) = cur.strip_prefix(p) {
                cur = rest.trim_start().to_string();
            }
        }
        if cur.len() == before {
            break;
        }
    }
    cur.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_normalization() {
        assert_eq!(normalize_subject("Re: RE: Fwd:  Q3   plan "), "q3 plan");
        assert_eq!(normalize_subject("AW: Angebot"), "angebot");
        assert_eq!(normalize_subject("Reply all"), "reply all");
        assert_eq!(normalize_subject(""), "");
    }
}
