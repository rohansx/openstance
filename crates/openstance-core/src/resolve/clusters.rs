//! Union-find over identity ids and resolution steps 1-13.
//! Spec: `docs/tech-spec/entity-resolution.md` section 5 (P1-CORE-07).
//!
//! Owned by the "resolve" stream. `union(a, b)` always makes the smaller
//! root the parent, records `(rule, confidence)` on the absorbed root in a
//! never-compressed union tree, and checks `split_from` cannot-links first.
//! Order: `[me]` and `merged_into` must-links, rule 1 co-reference, rule 2
//! alias keys, rule 3 name+company (with the large-company guard), rule 4
//! name+thread, enrichment identities (Phase 3), dormant filter.
//!
//! Every collection that is iterated is ordered (`BTreeMap`/`BTreeSet`/
//! sorted `Vec`), so the result is a pure function of the database rows,
//! config and data files.

use std::collections::{BTreeMap, BTreeSet};

use openstance_import::HandleKind;
use rusqlite::Connection;

use super::normalize::{alias_key, is_matchable, name_key, normalize_handle, NormalizeContext};
use super::rules::DataFiles;
use super::{Assignment, MatchRule, Resolution};
use crate::config::Config;
use crate::error::Result;
use crate::IdentityId;

/// More distinct name keys than this on one handle make it a multi-name
/// handle whose names are ignored for matching.
const MULTI_NAME_LIMIT: usize = 3;

/// Union-find with a union tree that remembers why each union happened.
#[derive(Debug, Clone, Default)]
pub struct UnionFind {
    parent: BTreeMap<IdentityId, IdentityId>,
    /// absorbed root -> (new root, rule, confidence); never compressed.
    tree: BTreeMap<IdentityId, (IdentityId, MatchRule, f64)>,
    /// cannot-link partners per identity (symmetric).
    cannot: BTreeMap<IdentityId, BTreeSet<IdentityId>>,
    /// cannot-link partners per current root (merged on union).
    root_cannot: BTreeMap<IdentityId, BTreeSet<IdentityId>>,
    /// Unions refused because of a cannot-link.
    pub blocked: u64,
}

impl UnionFind {
    /// Root (anchor) of `x`.
    pub fn find(&mut self, x: IdentityId) -> IdentityId {
        let mut root = x;
        while let Some(&p) = self.parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        // Path compression on the find structure only.
        let mut cur = x;
        while cur != root {
            let next = self.parent.get(&cur).copied().unwrap_or(root);
            self.parent.insert(cur, root);
            cur = next;
        }
        root
    }

    /// Record a cannot-link between two identities.
    pub fn cannot_link(&mut self, a: IdentityId, b: IdentityId) {
        if a == b {
            return;
        }
        self.cannot.entry(a).or_default().insert(b);
        self.cannot.entry(b).or_default().insert(a);
        let (ra, rb) = (self.find(a), self.find(b));
        self.root_cannot.entry(ra).or_default().insert(b);
        self.root_cannot.entry(rb).or_default().insert(a);
    }

    /// True when the clusters rooted at `ra` and `rb` contain a
    /// cannot-linked pair. Links are symmetric, so a block needs partner
    /// lists on both sides; the smaller list is scanned.
    fn blocked_between(&mut self, ra: IdentityId, rb: IdentityId) -> bool {
        let (Some(x), Some(y)) = (self.root_cannot.get(&ra), self.root_cannot.get(&rb)) else {
            return false;
        };
        let (side, other) = if x.len() <= y.len() {
            (ra, rb)
        } else {
            (rb, ra)
        };
        let partners: Vec<IdentityId> = self.root_cannot[&side].iter().copied().collect();
        partners.into_iter().any(|p| self.find(p) == other)
    }

    /// Union the clusters of `a` and `b`. Returns `false` when they were
    /// already one cluster or a cannot-link blocks the union (counted).
    pub fn union(&mut self, a: IdentityId, b: IdentityId, rule: MatchRule, conf: f64) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        if self.blocked_between(ra, rb) {
            self.blocked += 1;
            return false;
        }
        let (root, child) = if ra < rb { (ra, rb) } else { (rb, ra) };
        self.parent.insert(child, root);
        self.parent.entry(root).or_insert(root);
        self.tree.insert(child, (root, rule, conf));
        if let Some(moved) = self.root_cannot.remove(&child) {
            self.root_cannot.entry(root).or_default().extend(moved);
        }
        true
    }

    /// Confidence (min over the union-tree path) and rule of the weakest
    /// edge from `x` to its anchor; the anchor itself is `(Anchor, 1.0)`.
    pub fn path(&self, x: IdentityId) -> (MatchRule, f64) {
        let mut cur = x;
        let mut best: Option<(MatchRule, f64)> = None;
        while let Some(&(p, rule, conf)) = self.tree.get(&cur) {
            if best.is_none_or(|(_, c)| conf < c) {
                best = Some((rule, conf));
            }
            cur = p;
        }
        best.unwrap_or((MatchRule::Anchor, 1.0))
    }
}

/// What resolution reads from the raw tables, aggregated per identity.
/// Shared with the review queue (suggestions use the same keys).
#[derive(Debug, Clone, Default)]
pub(crate) struct Observations {
    /// Every identity: id -> (kind, value), ordered by id.
    pub identities: BTreeMap<IdentityId, (String, String)>,
    /// Matchable name keys per identity (multi-name and role handles empty).
    pub names: BTreeMap<IdentityId, BTreeSet<String>>,
    /// Identities with any name hint (or a name-bearing handle kind).
    pub has_name: BTreeSet<IdentityId>,
    /// Company keys per identity (hints + non-freemail email domain).
    pub companies: BTreeMap<IdentityId, BTreeSet<String>>,
    /// Co-reference groups `(interaction_id, slot)` with > 1 identity, in order.
    pub coref: Vec<Vec<IdentityId>>,
    /// Thread / meeting key -> participating identities.
    pub threads: BTreeMap<String, BTreeSet<IdentityId>>,
    /// Shared handles (role addresses, handles on several differently named cards).
    pub shared: BTreeSet<IdentityId>,
    /// Identities with at least one interaction of a kind other than `follow`.
    pub non_follow: BTreeSet<IdentityId>,
    /// Identities with any interaction at all.
    pub observed: BTreeSet<IdentityId>,
    /// Identities named by an override (`person_id`, or the value of
    /// `merged_into` / `split_from`).
    pub override_refs: BTreeSet<IdentityId>,
}

/// One `interaction_person` row with what resolution needs.
struct ObsRow {
    interaction_id: i64,
    slot: i64,
    identity_id: IdentityId,
    name_hint: Option<String>,
    company_hint: Option<String>,
    kind: String,
    thread_key: Option<String>,
    enrich: bool,
}

impl Observations {
    pub(crate) fn load(conn: &Connection, data: &DataFiles) -> Result<Observations> {
        let mut obs = Observations::default();
        let mut st = conn.prepare("SELECT id, kind, value FROM identity ORDER BY id")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, kind, value) = row?;
            obs.identities.insert(id, (kind, value));
        }

        let mut raw_names: BTreeMap<IdentityId, BTreeSet<String>> = BTreeMap::new();
        // card (interaction_id, slot) -> (identities, matchable names on the card)
        let mut card: Option<((i64, i64), BTreeSet<IdentityId>, BTreeSet<String>, bool)> = None;
        let mut card_names: BTreeMap<IdentityId, BTreeSet<String>> = BTreeMap::new();
        let mut card_count: BTreeMap<IdentityId, usize> = BTreeMap::new();

        let mut st = conn.prepare(
            "SELECT ip.interaction_id, ip.slot, ip.identity_id, ip.name_hint, ip.company_hint,
                    i.kind, i.thread_key, s.kind
             FROM interaction_person ip
             JOIN interaction i ON i.id = ip.interaction_id
             JOIN source s ON s.id = i.source_id
             ORDER BY ip.interaction_id, ip.slot, ip.identity_id",
        )?;
        let rows = st.query_map([], |r| {
            let source_kind: String = r.get(7)?;
            Ok(ObsRow {
                interaction_id: r.get(0)?,
                slot: r.get(1)?,
                identity_id: r.get(2)?,
                name_hint: r.get(3)?,
                company_hint: r.get(4)?,
                kind: r.get(5)?,
                thread_key: r.get(6)?,
                enrich: source_kind.starts_with("enrich:"),
            })
        })?;
        let mut flush =
            |card: Option<((i64, i64), BTreeSet<IdentityId>, BTreeSet<String>, bool)>,
             obs: &mut Observations| {
                if let Some((_, ids, names, enrich)) = card {
                    if ids.len() > 1 && !enrich {
                        for id in &ids {
                            card_names
                                .entry(*id)
                                .or_default()
                                .extend(names.iter().cloned());
                            *card_count.entry(*id).or_insert(0) += 1;
                        }
                        obs.coref.push(ids.into_iter().collect());
                    }
                }
            };
        for row in rows {
            let r = row?;
            obs.observed.insert(r.identity_id);
            if r.kind != "follow" {
                obs.non_follow.insert(r.identity_id);
            }
            let key = (r.interaction_id, r.slot);
            if card.as_ref().map(|c| c.0) != Some(key) {
                flush(card.take(), &mut obs);
                card = Some((key, BTreeSet::new(), BTreeSet::new(), r.enrich));
            }
            let mut matchable_name = None;
            if let Some(name) = r.name_hint.as_deref().filter(|n| !n.trim().is_empty()) {
                obs.has_name.insert(r.identity_id);
                let local_part = obs
                    .identities
                    .get(&r.identity_id)
                    .filter(|(k, _)| k == "email")
                    .and_then(|(_, v)| v.split('@').next());
                // A display name that is just the address's local part is a
                // display fallback, never a match key.
                let is_local_part = local_part.is_some_and(|l| name.trim().eq_ignore_ascii_case(l));
                let k = name_key(name);
                if !k.is_empty() && !is_local_part {
                    raw_names
                        .entry(r.identity_id)
                        .or_default()
                        .insert(k.clone());
                    if is_matchable(&k) {
                        matchable_name = Some(k);
                    }
                }
            }
            if let Some(c) = card.as_mut() {
                c.1.insert(r.identity_id);
                if let Some(k) = matchable_name {
                    c.2.insert(k);
                }
            }
            if let Some(company) = r.company_hint.as_deref() {
                if let Some(k) = data.company_hint_key(company) {
                    obs.companies.entry(r.identity_id).or_default().insert(k);
                }
            }
            match r.kind.as_str() {
                "email" => {
                    if let Some(t) = r.thread_key.as_deref().filter(|t| !t.is_empty()) {
                        obs.threads
                            .entry(format!("t:{t}"))
                            .or_default()
                            .insert(r.identity_id);
                    }
                }
                "meeting" => {
                    obs.threads
                        .entry(format!("m:{}", r.interaction_id))
                        .or_default()
                        .insert(r.identity_id);
                }
                _ => {}
            }
        }
        flush(card.take(), &mut obs);

        for (id, (kind, value)) in &obs.identities {
            match kind.as_str() {
                "email" => {
                    if let Some(k) = data.email_company_key(value) {
                        obs.companies.entry(*id).or_default().insert(k);
                    }
                    if data.is_role_address(value) {
                        obs.shared.insert(*id);
                    }
                }
                "name_company" | "name_local" => {
                    obs.has_name.insert(*id);
                }
                _ => {}
            }
        }
        // A handle on several cards that disagree on the name is shared.
        for (id, names) in &card_names {
            if card_count.get(id).copied().unwrap_or(0) > 1 && names.len() > 1 {
                obs.shared.insert(*id);
            }
        }
        // Matchable names, ignoring multi-name and shared handles.
        for (id, keys) in raw_names {
            if keys.len() > MULTI_NAME_LIMIT || obs.shared.contains(&id) {
                continue;
            }
            let m: BTreeSet<String> = keys.into_iter().filter(|k| is_matchable(k)).collect();
            if !m.is_empty() {
                obs.names.insert(id, m);
            }
        }

        let mut st = conn.prepare(
            "SELECT person_id, field, value FROM override
             WHERE field IN ('merged_into', 'split_from') OR person_id IN (SELECT id FROM identity)",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (person, field, value) = row?;
            obs.override_refs.insert(person);
            if field == "merged_into" || field == "split_from" {
                if let Some(v) = value.and_then(|v| v.trim().parse::<i64>().ok()) {
                    obs.override_refs.insert(v);
                }
            }
        }
        Ok(obs)
    }
}

/// Cluster attributes used by rules 3 and 4.
#[derive(Debug, Clone, Default)]
struct Attrs {
    names: BTreeSet<String>,
    companies: BTreeSet<String>,
}

fn merge_attrs(
    attrs: &mut BTreeMap<IdentityId, Attrs>,
    uf: &mut UnionFind,
    a: IdentityId,
    b: IdentityId,
) {
    let root = uf.find(a);
    debug_assert_eq!(root, uf.find(b));
    for old in [a, b] {
        if old != root {
            if let Some(moved) = attrs.remove(&old) {
                let t = attrs.entry(root).or_default();
                t.names.extend(moved.names);
                t.companies.extend(moved.companies);
            }
        }
    }
}

/// `[me] identities` resolved to identity ids (missing handles skipped).
pub(crate) fn me_identity_ids(conn: &Connection, cfg: &Config) -> Result<Vec<IdentityId>> {
    let ctx = NormalizeContext {
        default_region: Some(cfg.import.default_region.clone()).filter(|r| !r.trim().is_empty()),
        importer_kind: String::new(),
    };
    let mut out = Vec::new();
    let mut st = conn.prepare_cached("SELECT id FROM identity WHERE kind = ?1 AND value = ?2")?;
    for spec in &cfg.me.identities {
        let (kind, raw) = match spec.split_once(':') {
            Some((k, v)) if HandleKind::parse(k.trim()).is_some() => (k.trim(), v),
            _ if spec.contains('@') => ("email", spec.as_str()),
            _ => continue,
        };
        let Some(hk) = HandleKind::parse(kind) else {
            continue;
        };
        let Some(value) = normalize_handle(hk, raw, &ctx) else {
            continue;
        };
        let mut rows = st.query([kind, value.as_str()])?;
        if let Some(row) = rows.next()? {
            out.push(row.get(0)?);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// Steps 1-13 over the whole database (read-only).
pub fn resolve(conn: &Connection, cfg: &Config) -> Result<Resolution> {
    let data = DataFiles::load(&cfg.resolution);
    let obs = Observations::load(conn, &data)?;
    let mut uf = UnionFind::default();
    for &id in obs.identities.keys() {
        uf.parent.insert(id, id);
    }
    let exists = |id: IdentityId| obs.identities.contains_key(&id);

    // Cannot-links.
    let mut st = conn.prepare(
        "SELECT person_id, value FROM override WHERE field = 'split_from' ORDER BY set_at, id",
    )?;
    let splits = st
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (a, b) in splits {
        if let Some(b) = b.and_then(|v| v.trim().parse::<i64>().ok()) {
            if exists(a) && exists(b) {
                uf.cannot_link(a, b);
            }
        }
    }

    // Step 6: must-links. [me] first, then merged_into in (set_at, id) order.
    let me = me_identity_ids(conn, cfg)?;
    for w in me.windows(2) {
        uf.union(w[0], w[1], MatchRule::Override, 1.0);
    }
    let mut conflicting = Vec::new();
    let mut st = conn.prepare(
        "SELECT id, person_id, value FROM override WHERE field = 'merged_into' ORDER BY set_at, id",
    )?;
    let merges = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (oid, a, b) in merges {
        let Some(b) = b.and_then(|v| v.trim().parse::<i64>().ok()) else {
            continue;
        };
        if !exists(a) || !exists(b) {
            continue;
        }
        let before = uf.blocked;
        uf.union(a, b, MatchRule::Override, 1.0);
        if uf.blocked > before {
            conflicting.push(oid);
        }
    }

    // Step 7: rule 1, co-reference of non-shared handles.
    for group in &obs.coref {
        let members: Vec<IdentityId> = group
            .iter()
            .copied()
            .filter(|id| !obs.shared.contains(id))
            .collect();
        for w in members.windows(2) {
            uf.union(w[0], w[1], MatchRule::Exact, MatchRule::Exact.confidence());
        }
    }
    // A shared handle attaches only when exactly one cluster claims it.
    let mut claims: BTreeMap<IdentityId, BTreeSet<IdentityId>> = BTreeMap::new();
    for group in &obs.coref {
        for &s in group.iter().filter(|id| obs.shared.contains(id)) {
            let roots: Vec<IdentityId> = group
                .iter()
                .copied()
                .filter(|id| !obs.shared.contains(id))
                .collect();
            let e = claims.entry(s).or_default();
            for id in roots {
                e.insert(uf.find(id));
            }
        }
    }
    for (s, roots) in claims {
        if roots.len() == 1 {
            let r = *roots.iter().next().unwrap_or(&s);
            uf.union(s, r, MatchRule::Exact, MatchRule::Exact.confidence());
        }
    }

    // Step 8: rule 2, alias keys; groups ordered by min id.
    let mut by_alias: BTreeMap<String, Vec<IdentityId>> = BTreeMap::new();
    for (id, (kind, value)) in &obs.identities {
        if kind == "email" {
            by_alias
                .entry(alias_key(value, &data.gmail_domains))
                .or_default()
                .push(*id);
        }
    }
    let mut groups: Vec<Vec<IdentityId>> = by_alias.into_values().filter(|g| g.len() > 1).collect();
    groups.sort_by_key(|g| g[0]);
    for g in groups {
        for w in g.windows(2) {
            uf.union(
                w[0],
                w[1],
                MatchRule::GmailAlias,
                MatchRule::GmailAlias.confidence(),
            );
        }
    }

    // Step 9: cluster attributes.
    let ids: Vec<IdentityId> = obs.identities.keys().copied().collect();
    let mut attrs: BTreeMap<IdentityId, Attrs> = BTreeMap::new();
    for &id in &ids {
        let root = uf.find(id);
        let a = attrs.entry(root).or_default();
        if let Some(n) = obs.names.get(&id) {
            a.names.extend(n.iter().cloned());
        }
        if let Some(c) = obs.companies.get(&id) {
            a.companies.extend(c.iter().cloned());
        }
    }
    let mut company_size: BTreeMap<String, u32> = BTreeMap::new();
    for a in attrs.values() {
        for c in &a.companies {
            *company_size.entry(c.clone()).or_insert(0) += 1;
        }
    }

    // Step 10: rule 3, name + company.
    let mut name_index: BTreeMap<String, BTreeSet<IdentityId>> = BTreeMap::new();
    for (root, a) in &attrs {
        for n in &a.names {
            name_index.entry(n.clone()).or_default().insert(*root);
        }
    }
    for (name, roots) in &name_index {
        let common = data.is_common_name(name);
        let mut list: Vec<IdentityId> = roots.iter().map(|&r| uf.find(r)).collect();
        list.sort_unstable();
        list.dedup();
        for i in 1..list.len() {
            for j in 0..i {
                let (rc, re) = (uf.find(list[i]), uf.find(list[j]));
                if rc == re {
                    break;
                }
                let shared: Vec<&String> = match (attrs.get(&rc), attrs.get(&re)) {
                    (Some(x), Some(y)) => x.companies.intersection(&y.companies).collect(),
                    _ => Vec::new(),
                };
                if shared.is_empty() {
                    continue;
                }
                let guarded = common
                    && shared.iter().all(|k| {
                        company_size.get(*k).copied().unwrap_or(0) >= data.large_company_threshold
                    });
                if guarded {
                    continue;
                }
                if uf.union(
                    rc,
                    re,
                    MatchRule::NameCompany,
                    MatchRule::NameCompany.confidence(),
                ) {
                    merge_attrs(&mut attrs, &mut uf, rc, re);
                    break;
                }
            }
        }
    }

    // Step 11: rule 4, name + mutual thread / meeting.
    for members in obs.threads.values() {
        let mut roots: Vec<IdentityId> = members.iter().map(|&id| uf.find(id)).collect();
        roots.sort_unstable();
        roots.dedup();
        if roots.len() < 2 {
            continue;
        }
        let mut by_name: BTreeMap<String, Vec<IdentityId>> = BTreeMap::new();
        for r in &roots {
            if let Some(a) = attrs.get(r) {
                for n in &a.names {
                    by_name.entry(n.clone()).or_default().push(*r);
                }
            }
        }
        for group in by_name.values() {
            if group.len() < 2 {
                continue;
            }
            let first = group[0];
            for &other in &group[1..] {
                let (ra, rb) = (uf.find(first), uf.find(other));
                if ra != rb
                    && uf.union(
                        ra,
                        rb,
                        MatchRule::NameThread,
                        MatchRule::NameThread.confidence(),
                    )
                {
                    merge_attrs(&mut attrs, &mut uf, ra, rb);
                }
            }
        }
    }

    // Step 12: enrichment identities land in Phase 3.

    // Step 13: dormant filter.
    let mut cluster: BTreeMap<IdentityId, Vec<IdentityId>> = BTreeMap::new();
    for &id in &ids {
        let r = uf.find(id);
        cluster.entry(r).or_default().push(id);
    }
    let mut assignments = BTreeMap::new();
    for (root, members) in &cluster {
        let named = members.iter().any(|m| obs.has_name.contains(m));
        let observed = members.iter().any(|m| obs.observed.contains(m));
        let non_follow = members.iter().any(|m| obs.non_follow.contains(m));
        let overridden = members.iter().any(|m| obs.override_refs.contains(m))
            || members.iter().any(|m| me.contains(m));
        let dormant = if observed {
            !named && !non_follow && !overridden
        } else {
            !overridden
        };
        if dormant {
            continue;
        }
        for &m in members {
            let (rule, confidence) = uf.path(m);
            assignments.insert(
                m,
                Assignment {
                    person_id: *root,
                    confidence,
                    rule,
                },
            );
        }
    }
    let self_person = me
        .iter()
        .find_map(|id| assignments.get(id).map(|a: &Assignment| a.person_id));
    Ok(Resolution {
        assignments,
        self_person,
        blocked_by_override: uf.blocked,
        conflicting_overrides: conflicting,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_keeps_the_smaller_root_and_records_the_edge() {
        let mut uf = UnionFind::default();
        assert!(uf.union(5, 3, MatchRule::Exact, 1.0));
        assert!(uf.union(9, 5, MatchRule::NameThread, 0.75));
        assert_eq!(uf.find(9), 3);
        assert_eq!(uf.path(3), (MatchRule::Anchor, 1.0));
        assert_eq!(uf.path(5), (MatchRule::Exact, 1.0));
        assert_eq!(uf.path(9), (MatchRule::NameThread, 0.75));
        assert!(!uf.union(3, 9, MatchRule::Exact, 1.0));
    }

    #[test]
    fn weakest_edge_on_the_path_wins() {
        let mut uf = UnionFind::default();
        uf.union(7, 8, MatchRule::Exact, 1.0); // 8 -> 7
        uf.union(2, 7, MatchRule::NameCompany, 0.85); // 7 -> 2
        assert_eq!(uf.path(8), (MatchRule::NameCompany, 0.85));
    }

    #[test]
    fn cannot_links_block_unions_through_other_members() {
        let mut uf = UnionFind::default();
        uf.cannot_link(1, 4);
        assert!(uf.union(1, 2, MatchRule::Exact, 1.0));
        assert!(uf.union(3, 4, MatchRule::Exact, 1.0));
        assert!(!uf.union(2, 3, MatchRule::Override, 1.0));
        assert_eq!(uf.blocked, 1);
        assert_ne!(uf.find(2), uf.find(3));
    }
}
