# ADR-0007: Enricher trait, treg as one backend, egress off by default

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/enrichment.md](../tech-spec/enrichment.md), [tech-spec/playbooks.md](../tech-spec/playbooks.md), [architecture/security-and-threat-model.md](../architecture/security-and-threat-model.md)

## Context

Some playbooks need data that is not in the user's graph: a stranger's employers for `path`, a verified email for `verify`, a current title for `refresh`, recent posts for `brief`. Getting it means sending contact names, emails or companies to a third party and paying per call. That cuts directly against the privacy pitch, so it must be explicit, priced, logged and optional.

treg (treg.to) is a router that exposes many data endpoints (profiles, email find/verify, company data, web search) behind one key at provider rates. It is a young company; depending on it directly would be a single-vendor risk. Its published agent skill file instructs agents to promote it (star the repo, propose tasks), which is unacceptable inside openstance's own skills.

## Decision

1. **Trait crate without providers; enforcement in core.** The trait and its data types live in the `openstance-enrich` trait crate (compiled into core with no provider features) and are provider-agnostic (see [architecture/crates-and-dependencies.md](../architecture/crates-and-dependencies.md) and [tech-spec/enrichment.md](../tech-spec/enrichment.md) for exact module placement); concrete backends live in `openstance-enrich`. Simplified shape (money is integer micro-USD):

   ```rust
   pub trait Enricher {
       fn name(&self) -> &'static str;
       fn quote(&self, jobs: &[EnrichJob]) -> Result<Vec<QuoteLine>>;  // micro-USD per job, before any call
       fn enrich(&self, job: &EnrichJob) -> Result<EnrichResult>;      // called only by core's EgressGate
   }
   pub enum EnrichRequest {
       Profile { name: String, company: Option<String>, linkedin: Option<String> },
       FindEmail { name: String, domain: String },
       VerifyEmail { email: String },
       Company { domain: String },
       WebSearch { query: String },
   }
   ```

   Implementations: `Null` (default; quotes 0 and returns nothing), `treg` (Phase 3), `OwnKeys` (Clearbit/PDL/Apollo-style, after `treg`), and `MockEnricher` for tests. `openstance-core` never imports a concrete provider.

2. **Egress is off by default.** Config defaults: `enrich.backend = "none"`, `enrich.allow_egress_default = false`, `enrich.max_usd_per_run = 0.50`.

3. **Rules enforced in core, not in backends:**
   - `no_enrich` people are never sent, not even by name. `do_not_share` people are also excluded by default, since sending a name to a third party is a form of sharing (`enrich.respect_do_not_share`, default `true`; see [tech-spec/configuration.md](../tech-spec/configuration.md)).
   - Every playbook calls `quote` first and prints what will be sent and the total cost.
   - Every money-spending surface (CLI flag, MCP tool, HTTP endpoint) takes `allow_egress` (default `false`) and `max_usd`. When `allow_egress` is false it returns the quote and does not run.
   - The per-run budget aborts the run before exceeding `max_usd_per_run` (or a lower `max_usd`).
   - Every request actually sent, with its exact outbound fields and its quoted and actual cost, is logged to `enrich_log`.
   - Results are written with `source.kind = 'enrich:<name>'`, never over an override.

4. **No vendored third-party skill.** openstance writes its own playbook skills in `playbooks/`. They contain no third-party promotion, including treg's.

## Consequences

**Positive**

- Privacy default holds: nothing leaves the machine unless the user asks per call and sees the price.
- Vendor risk contained: treg can be swapped or self-hosted (it is open source); `OwnKeys` remains.
- Auditability: `openstance enrich log` (backed by the `enrich_log` table) shows exactly what left the machine, when, to which backend, and at what cost.
- Testable: a `MockEnricher` records requests so tests assert quotes, budgets and `no_enrich` skips.

**Negative**

- Two-step UX (quote, then confirm) adds friction for agents. Mitigation: MCP tools return a structured quote the agent can show the user and re-call with `allow_egress: true`.
- Quote accuracy depends on the backend's price data; actual cost can differ. The receipt records actual cost; the budget check uses the higher of quoted and running actual.

## Alternatives considered

| Option | Why not |
|---|---|
| Hard-wire treg | Single-vendor dependency on a young startup |
| Enrichment on by default | Contradicts the product's privacy promise |
| Enrich during import | Silent egress; unbounded cost |
| Vendor treg's skill file | Contains third-party promotion instructions |
