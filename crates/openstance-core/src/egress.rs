//! The egress gate (P3-CORE-01): the single choke point between a playbook
//! and an [`Enricher`]. Spec: `docs/tech-spec/enrichment.md` sections 4 to 7
//! and `docs/plan/phase-3.md` (P3-CORE-01 / P3-CORE-02).
//!
//! A backend can be buggy or hostile; the gate still guarantees that:
//!
//! 1. `no_enrich` people (and `do_not_share` people while
//!    `enrich.respect_do_not_share` is on) are dropped before the backend
//!    sees anything, and checked again right before each send;
//! 2. every job is priced ([`EgressGate::quote`]) before any call, and the
//!    price is written to `enrich_log` with status `quoted`;
//! 3. nothing is sent unless the caller approved that exact quote
//!    ([`EgressGate::approve`], i.e. `allow_egress = true`);
//! 4. a job whose price would push the run past its budget is not sent
//!    (status `blocked`, receipt `skipped: over_budget`);
//! 5. each attempt moves its `enrich_log` row `quoted -> sent ->
//!    ok | not_found | error` (or `blocked`), the row is written before the
//!    result is used, and the actual cost reported by the backend is kept;
//! 6. results are committed ([`EgressGate::commit`]) under one
//!    `source(kind = 'enrich:<backend>')` row and become field facts that
//!    rank below overrides ([`facts`]).
//!
//! The gate is sync and only needs `&Graph`: log rows are single autocommit
//! statements and [`EgressGate::commit`] opens its own `IMMEDIATE`
//! transaction.

pub mod facts;
pub mod log;

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::config::EnrichSection;
use crate::enrich::{
    usd_to_micro, EnrichData, EnrichError, EnrichJob, EnrichResult, Enricher, MicroUsd, Quote,
    QuoteLine, SkipReason, Skipped,
};
use crate::error::{Error, Result};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::{Graph, PersonId, SourceId};

/// Who is asking, for the budget rule (enrichment.md section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// A human at the CLI may raise the budget for one run with `--max-usd`.
    Cli,
    /// MCP / HTTP callers can only lower it: `min(max_usd, max_usd_per_run)`.
    Agent,
}

/// The effective budget of one run in micro-USD.
pub fn effective_budget(cfg: &EnrichSection, max_usd: Option<f64>, surface: Surface) -> MicroUsd {
    let ceiling = usd_to_micro(cfg.max_usd_per_run);
    match (max_usd.map(usd_to_micro), surface) {
        (None, _) => ceiling,
        (Some(m), Surface::Cli) => m,
        (Some(m), Surface::Agent) => m.min(ceiling),
    }
}

/// A new run id: 26 Crockford base32 characters, time-ordered (ULID layout:
/// 48 bits of milliseconds, then 80 bits that differ per call).
pub fn new_run_id(now_secs: i64) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ms = u128::try_from(now_secs.max(0)).unwrap_or(0) * 1000 + (nanos / 1_000_000) % 1000;
    let mut h = blake3::Hasher::new();
    h.update(&nanos.to_le_bytes());
    h.update(&std::process::id().to_le_bytes());
    h.update(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let rnd = h.finalize();
    let mut tail = [0u8; 16];
    tail.copy_from_slice(&rnd.as_bytes()[..16]);
    let value: u128 = ((ms & 0xFFFF_FFFF_FFFF) << 80) | (u128::from_le_bytes(tail) >> 48);
    (0..26)
        .rev()
        .map(|i| ALPHABET[((value >> (i * 5)) & 31) as usize] as char)
        .collect()
}

/// What a run cost (playbooks.md section 1, interfaces.md 1.9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    /// Run id shared by every `enrich_log` row of the run.
    pub run_id: String,
    /// Backend id.
    pub backend: String,
    /// Requests that left the machine.
    pub requests_sent: u32,
    /// Total of the quote.
    pub quoted_micro_usd: MicroUsd,
    /// Actual spend reported by the backend.
    pub spent_micro_usd: MicroUsd,
    /// Effective budget.
    pub budget_micro_usd: MicroUsd,
    /// Jobs not sent, and why.
    pub skipped: Vec<Skipped>,
    /// Every `enrich_log` row this run wrote.
    pub enrich_log_ids: Vec<i64>,
}

/// The result of [`EgressGate::execute`].
#[derive(Debug)]
pub enum Executed {
    /// The backend answered (possibly `NotFound`, which may still cost money).
    Done {
        /// The `enrich_log` row.
        log_id: i64,
        /// The result.
        result: Box<EnrichResult>,
    },
    /// The gate did not send the job.
    Blocked {
        /// The `enrich_log` row marked `blocked`, if the job had one.
        log_id: Option<i64>,
        /// Why.
        reason: SkipReason,
    },
    /// The backend failed. `fatal` errors (auth, payment, config, disabled)
    /// stop every later job of the run.
    Failed {
        /// The `enrich_log` row.
        log_id: i64,
        /// Error message (never contains the token).
        error: String,
        /// The run cannot continue.
        fatal: bool,
    },
}

/// What [`EgressGate::commit`] wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommitStats {
    /// The `enrich:<backend>` source row (None when nothing was committed).
    pub source_id: Option<SourceId>,
    /// `enrich_log` rows attached to it.
    pub rows: u32,
    /// People whose fields were recomputed.
    pub people: Vec<PersonId>,
}

/// A priced job and its `enrich_log` row.
#[derive(Debug, Clone)]
struct Slot {
    line: QuoteLine,
    log_id: i64,
    used: bool,
}

/// See the module docs.
pub struct EgressGate<'a> {
    g: &'a Graph,
    enricher: &'a dyn Enricher,
    run_id: String,
    playbook: String,
    budget: MicroUsd,
    spent: MicroUsd,
    quote: Option<Quote>,
    slots: Vec<Slot>,
    approved: bool,
    sent: u32,
    skipped: Vec<Skipped>,
    log_ids: Vec<i64>,
    fatal: Option<String>,
    done: Vec<(i64, Option<PersonId>)>,
}

impl std::fmt::Debug for EgressGate<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EgressGate")
            .field("backend", &self.enricher.name())
            .field("run_id", &self.run_id)
            .field("playbook", &self.playbook)
            .field("budget", &self.budget)
            .field("spent", &self.spent)
            .field("approved", &self.approved)
            .finish_non_exhaustive()
    }
}

impl<'a> EgressGate<'a> {
    /// A gate for one run of `playbook` with a resolved budget.
    pub fn new(
        g: &'a Graph,
        enricher: &'a dyn Enricher,
        playbook: &str,
        run_id: impl Into<String>,
        budget: MicroUsd,
    ) -> Self {
        EgressGate {
            g,
            enricher,
            run_id: run_id.into(),
            playbook: playbook.to_string(),
            budget,
            spent: 0,
            quote: None,
            slots: Vec::new(),
            approved: false,
            sent: 0,
            skipped: Vec::new(),
            log_ids: Vec::new(),
            fatal: None,
            done: Vec::new(),
        }
    }

    /// The backend id.
    pub fn backend(&self) -> &'static str {
        self.enricher.name()
    }

    /// The run id.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// The effective budget.
    pub fn budget(&self) -> MicroUsd {
        self.budget
    }

    /// Actual spend so far.
    pub fn spent(&self) -> MicroUsd {
        self.spent
    }

    /// The current quote, if [`EgressGate::quote`] ran.
    pub fn current_quote(&self) -> Option<&Quote> {
        self.quote.as_ref()
    }

    fn conn(&self) -> &Connection {
        self.g.conn()
    }

    /// Why `person` must not be sent, if anything (re-read every time: the
    /// flag can change between the quote and the send).
    fn privacy_block(&self, person: Option<PersonId>) -> Result<Option<SkipReason>> {
        let Some(p) = person else { return Ok(None) };
        let current = match resolve_person(self.conn(), &PersonKey::Id(p)) {
            Ok(r) => r.person,
            Err(Error::NotFound(_)) => p,
            Err(e) => return Err(e),
        };
        let cluster = crate::overrides::cluster(self.conn(), current)?;
        let flags: Option<(bool, bool)> = self
            .conn()
            .query_row(
                "SELECT no_enrich, do_not_share FROM person WHERE id = ?1",
                [current],
                |r| Ok((r.get::<_, i64>(0)? != 0, r.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        let (row_ne, row_dns) = flags.unwrap_or((false, false));
        let ne = row_ne
            || crate::overrides::privacy_flag(
                self.conn(),
                &cluster,
                crate::overrides::OverrideField::NoEnrich,
            )?;
        if ne {
            return Ok(Some(SkipReason::NoEnrich));
        }
        let respect = self.g.config().enrich.respect_do_not_share;
        let dns = row_dns
            || crate::overrides::privacy_flag(
                self.conn(),
                &cluster,
                crate::overrides::OverrideField::DoNotShare,
            )?;
        Ok((respect && dns).then_some(SkipReason::DoNotShare))
    }

    /// Filter (`no_enrich`, `do_not_share`, unsupported ops), price what is
    /// left, compare it to the budget, and write one `quoted` row per priced
    /// job. Sends no contact data. Replaces any earlier quote of this gate.
    pub fn quote(&mut self, jobs: Vec<EnrichJob>) -> Result<Quote> {
        self.approved = false;
        self.slots.clear();
        let mut skipped = Vec::new();
        let mut keep = Vec::new();
        for job in jobs {
            let op = job.request.op();
            let reason = if let Some(r) = self.privacy_block(job.person_id)? {
                Some(r)
            } else if self.enricher.name() == "none" {
                Some(SkipReason::BackendNone)
            } else if !self.enricher.supports(op) {
                Some(SkipReason::Unsupported)
            } else {
                None
            };
            match reason {
                Some(reason) => skipped.push(Skipped {
                    person_id: job.person_id,
                    op: op.to_string(),
                    reason,
                }),
                None => keep.push(job),
            }
        }
        let lines = if keep.is_empty() {
            Vec::new()
        } else {
            self.enricher
                .quote(&keep)
                .map_err(|e| Error::BadRequest(format!("could not price the request: {e}")))?
        };
        let now = self.g.now();
        for line in &lines {
            let id = self.insert_row(&line.job, line.micro_usd, now)?;
            if !self.log_ids.contains(&id) {
                self.log_ids.push(id);
            }
            self.slots.push(Slot {
                line: line.clone(),
                log_id: id,
                used: false,
            });
        }
        let total = QuoteLine::total(&lines);
        self.skipped.extend(skipped.iter().cloned());
        let q = Quote {
            backend: self.enricher.name().to_string(),
            lines,
            total_micro_usd: total,
            budget_micro_usd: self.budget,
            within_budget: total <= self.budget,
            skipped,
            quoted_at: now,
        };
        self.quote = Some(q.clone());
        Ok(q)
    }

    /// The `quoted` row of `job`: an unused one this run already wrote for
    /// the same request (the CLI quotes, asks, then runs with the same run
    /// id), or a new one.
    fn insert_row(&self, job: &EnrichJob, quoted: MicroUsd, now: i64) -> Result<i64> {
        let request = outbound_json(job);
        let price = i64::try_from(quoted).unwrap_or(i64::MAX);
        let taken: Vec<i64> = self.slots.iter().map(|s| s.log_id).collect();
        let mut st = self.conn().prepare_cached(
            "SELECT id FROM enrich_log
             WHERE run_id = ?1 AND backend = ?2 AND status = 'quoted' AND op = ?3
               AND person_id IS ?4 AND request_json = ?5
             ORDER BY id",
        )?;
        let existing: Vec<i64> = st
            .query_map(
                params![
                    self.run_id,
                    self.enricher.name(),
                    job.request.op(),
                    job.person_id,
                    request
                ],
                |r| r.get(0),
            )?
            .collect::<rusqlite::Result<_>>()?;
        if let Some(id) = existing.into_iter().find(|id| !taken.contains(id)) {
            self.conn().execute(
                "UPDATE enrich_log SET at = ?2, quoted_micro_usd = ?3 WHERE id = ?1",
                params![id, now, price],
            )?;
            return Ok(id);
        }
        self.conn().execute(
            "INSERT INTO enrich_log(run_id, at, backend, playbook, person_id, op, request_json,
                quoted_micro_usd, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'quoted')",
            params![
                self.run_id,
                now,
                self.enricher.name(),
                self.playbook,
                job.person_id,
                job.request.op(),
                request,
                price,
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Record that the caller allowed this exact quote. An approval covers
    /// one quote and is lost when the gate quotes again.
    pub fn approve(&mut self, q: &Quote) -> Result<()> {
        match &self.quote {
            Some(cur) if cur == q => {
                self.approved = true;
                Ok(())
            }
            _ => Err(Error::BadRequest(
                "egress approval does not match the current quote".into(),
            )),
        }
    }

    /// Send one job of the approved quote. The job is matched to an unused
    /// quoted line with the same op and person; its fields may have been
    /// filled in by earlier steps (a domain from the profile, for example),
    /// and the row then records exactly what was sent.
    pub fn execute(&mut self, job: &EnrichJob) -> Result<Executed> {
        if !self.approved {
            return Err(Error::BadRequest(
                "egress not approved: quote first, then approve that quote".into(),
            ));
        }
        let op = job.request.op();
        let Some(i) = self.slots.iter().position(|s| {
            !s.used && s.line.job.request.op() == op && s.line.job.person_id == job.person_id
        }) else {
            return Err(Error::BadRequest(format!(
                "{op} request was not in the approved quote"
            )));
        };
        self.slots[i].used = true;
        let slot = self.slots[i].clone();
        let skip = |reason| Skipped {
            person_id: job.person_id,
            op: op.to_string(),
            reason,
        };
        if let Some(reason) = self.privacy_block(job.person_id)? {
            self.block(slot.log_id, reason, None)?;
            self.skipped.push(skip(reason));
            return Ok(Executed::Blocked {
                log_id: Some(slot.log_id),
                reason,
            });
        }
        if let Some(why) = self.fatal.clone() {
            self.block(slot.log_id, SkipReason::Unsupported, Some(&why))?;
            return Ok(Executed::Failed {
                log_id: slot.log_id,
                error: why,
                fatal: true,
            });
        }
        if self.spent.saturating_add(slot.line.micro_usd) > self.budget {
            self.block(slot.log_id, SkipReason::OverBudget, None)?;
            self.skipped.push(skip(SkipReason::OverBudget));
            return Ok(Executed::Blocked {
                log_id: Some(slot.log_id),
                reason: SkipReason::OverBudget,
            });
        }
        // The row says `sent` before the request leaves.
        self.conn().execute(
            "UPDATE enrich_log SET status = 'sent', at = ?2, request_json = ?3 WHERE id = ?1",
            params![slot.log_id, self.g.now(), outbound_json(job)],
        )?;
        self.sent += 1;
        match self.enricher.enrich(job) {
            Ok(result) => {
                if result.cost_micro_usd > slot.line.micro_usd {
                    tracing::warn!(
                        op,
                        quoted = slot.line.micro_usd,
                        actual = result.cost_micro_usd,
                        "backend charged more than it quoted"
                    );
                }
                self.spent = self.spent.saturating_add(result.cost_micro_usd);
                let status = if matches!(result.data, EnrichData::NotFound) {
                    "not_found"
                } else {
                    "ok"
                };
                self.conn().execute(
                    "UPDATE enrich_log SET status = ?2, cost_micro_usd = ?3, provider = ?4,
                        response_json = ?5
                     WHERE id = ?1",
                    params![
                        slot.log_id,
                        status,
                        i64::try_from(result.cost_micro_usd).unwrap_or(i64::MAX),
                        result.provider,
                        serde_json::to_string(&result.data)?,
                    ],
                )?;
                if status == "ok" {
                    self.done.push((slot.log_id, job.person_id));
                }
                Ok(Executed::Done {
                    log_id: slot.log_id,
                    result: Box::new(result),
                })
            }
            Err(e) => {
                let fatal = matches!(
                    e,
                    EnrichError::Auth(_)
                        | EnrichError::Disabled
                        | EnrichError::Config(_)
                        | EnrichError::Payment(_)
                );
                let msg = e.to_string();
                self.conn().execute(
                    "UPDATE enrich_log SET status = 'error', error = ?2 WHERE id = ?1",
                    params![slot.log_id, msg],
                )?;
                if fatal {
                    self.fatal = Some(format!("run stopped after: {msg}"));
                }
                Ok(Executed::Failed {
                    log_id: slot.log_id,
                    error: msg,
                    fatal,
                })
            }
        }
    }

    fn block(&self, log_id: i64, reason: SkipReason, msg: Option<&str>) -> Result<()> {
        let why = match msg {
            Some(m) => m.to_string(),
            None => serde_json::to_value(reason)?
                .as_str()
                .unwrap_or("blocked")
                .to_string(),
        };
        self.conn().execute(
            "UPDATE enrich_log SET status = 'blocked', error = ?2 WHERE id = ?1",
            params![log_id, why],
        )?;
        Ok(())
    }

    /// Persist this run's successful results about people under one
    /// `source(kind = 'enrich:<backend>')` row and recompute those people.
    /// Overrides keep winning ([`facts`]); `rebuild` replays the same rows.
    pub fn commit(&mut self) -> Result<CommitStats> {
        let rows: Vec<(i64, PersonId)> = self
            .done
            .iter()
            .filter_map(|(id, p)| p.map(|p| (*id, p)))
            .collect();
        if rows.is_empty() {
            return Ok(CommitStats::default());
        }
        let conn = self.g.conn();
        let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        let now = self.g.now();
        tx.execute(
            "INSERT INTO source(kind, label, imported_at) VALUES (?1, ?2, ?3)",
            params![
                format!("enrich:{}", self.enricher.name()),
                format!("{} {}", self.playbook, self.run_id),
                now
            ],
        )?;
        let source_id = tx.last_insert_rowid();
        let mut people = Vec::new();
        for (log_id, person) in &rows {
            tx.execute(
                "UPDATE enrich_log SET source_id = ?2 WHERE id = ?1",
                params![log_id, source_id],
            )?;
            let current = match resolve_person(&tx, &PersonKey::Id(*person)) {
                Ok(r) => Some(r.person),
                Err(Error::NotFound(_)) => None,
                Err(e) => return Err(e),
            };
            if let Some(c) = current {
                if !people.contains(&c) {
                    people.push(c);
                }
            }
        }
        people.sort_unstable();
        if !people.is_empty() {
            crate::rebuild::refresh_people(&tx, self.g.config(), &people, now)?;
        }
        tx.commit()?;
        self.done.clear();
        Ok(CommitStats {
            source_id: Some(source_id),
            rows: u32::try_from(rows.len()).unwrap_or(u32::MAX),
            people,
        })
    }

    /// The cost receipt so far.
    pub fn receipt(&self) -> Receipt {
        Receipt {
            run_id: self.run_id.clone(),
            backend: self.enricher.name().to_string(),
            requests_sent: self.sent,
            quoted_micro_usd: self.quote.as_ref().map_or(0, |q| q.total_micro_usd),
            spent_micro_usd: self.spent,
            budget_micro_usd: self.budget,
            skipped: self.skipped.clone(),
            enrich_log_ids: self.log_ids.clone(),
        }
    }
}

/// `{"kind": op, ...outbound fields}`: exactly what leaves the machine.
fn outbound_json(job: &EnrichJob) -> String {
    let mut m = serde_json::Map::new();
    m.insert("kind".into(), job.request.op().into());
    for (k, v) in job.request.outbound_fields() {
        m.insert(k.into(), v.into());
    }
    serde_json::Value::Object(m).to_string()
}

#[cfg(test)]
mod tests;
