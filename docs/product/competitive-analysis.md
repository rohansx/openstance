# Competitive analysis

> Status: working document. Owner: product. Last reviewed: 2026-10-20.
> Sources: the v0.1 research (`dist/research.md`, cited vendor pages retrieved October 2026) and the product overview (Happenstance figures "from their own compare page and MCP docs as of Sep 2026"). **Where the two disagree, this document follows the research**, which was checked against vendor pages and is more cautious.

## Ground rules for anything we publish

These rules apply to this document, the README, the launch post ([../plan/launch.md](../plan/launch.md)) and any comparison page.

1. **Factual, sourced, dated.** Every claim about another product links to the vendor's own page and states the retrieval date. If we cannot source it, we do not publish it.
2. **No parody, no borrowed branding.** We do not use a competitor's name, logo, colors or "open-X" framing in copy. The working name `openstance` itself carries this risk and is under review ([ADR-0001](../decisions/0001-product-name.md)).
3. **Unconfirmed figures are labeled.** The "about 15 searches per month free" Happenstance figure is third-party reported and **unconfirmed**; the vendor's current pricing page does not state a numeric free allowance. Do not present it as current fact.
4. **Do not claim a competitor lacks something it has.** In particular, do not claim Happenstance lacks privacy controls or match evidence. Its security page describes metadata-only processing and visibility controls, and its own listing says it attempts to provide concrete evidence. Our difference is **where the data and application live and how inspectable the evidence is**, not that others ignore privacy.
5. **Lead with what we do, not who we beat.** Launch copy is "free search, your graph as a file, agent-native", not "better Happenstance".

## Market map

```mermaid
quadrantChart
  title Where products sit (qualitative, not measured)
  x-axis Hosted vendor-held data --> Local user-held data
  y-axis Memory and reminders --> Discovery and intro paths
  quadrant-1 Local discovery (target)
  quadrant-2 Hosted discovery
  quadrant-3 Hosted personal CRM
  quadrant-4 Self-hosted personal CRM
  Happenstance: [0.15, 0.85]
  The Swarm: [0.1, 0.75]
  Village: [0.2, 0.8]
  Mesh: [0.2, 0.35]
  Dex: [0.25, 0.25]
  Monica: [0.85, 0.2]
  openstance: [0.9, 0.8]
```

Placement is a judgement for orientation only. Each element of openstance's combination (personal graph you hold, traceable evidence, quoted egress, open extension model, free agent access) exists somewhere else; the bet is that the combination, executed well, is worth switching for. That has to be shown with measured outcomes ([vision.md](vision.md#success-measures)), not asserted.

## Happenstance (primary reference)

Happenstance offers natural-language search across personal and extended professional networks, relationship ranking, warm-introduction paths, community groups that pool networks, drafted intro requests, an iPhone app, and API, MCP and CLI access. Sales, hiring, fundraising and job search are explicit use cases. These are existing capabilities; we do not claim to have invented any of them.

### Factual comparison

"Verified" means checked against the vendor's own page in the v0.1 research (October 2026). "Overview" means stated in our product overview from Sep 2026 vendor pages but not re-checked. "Unconfirmed" means third-party or not found on vendor pages.

| Dimension | Happenstance | Status | openstance |
|---|---|---|---|
| Hosting | Cloud service | Verified | Local binary and one SQLite file; optional cloud sync later ([pricing-and-cloud.md](pricing-and-cloud.md)) |
| Sources | Gmail, Calendar, Contacts, LinkedIn, Outlook, Twitter, Instagram connections | Verified | Exports: mbox (Takeout), LinkedIn CSV, vcf, ics, X archive in v1; IMAP; WhatsApp, Slack, Discord, Telegram, notes, PST later. OAuth optional and later ([../tech-spec/ingestion.md](../tech-spec/ingestion.md)) |
| Data processed | Communication metadata, not message bodies or attachments; previews network exports locally | Verified (security page) | Email headers only by default; bodies opt-in (`import.email_bodies`); all on the user's disk |
| Data use | Does not sell network data or train models on it | Verified (security page) | No server; nothing leaves the machine except quoted, logged enrichment calls |
| Visibility | Searchable by user, accepted friends and joined groups; a named network "constellation" on the public profile is visible by default and can be disabled | Verified (security page) | Nothing shared by default; no public profile; named views only, encrypted to a recipient key |
| Security posture | Advertises encryption and independent security reviews | Verified (security page); SOC 2 claim is Overview only | Local file is as safe as the disk (full-disk encryption recommended; optional SQLCipher build); see [../architecture/security-and-threat-model.md](../architecture/security-and-threat-model.md) |
| Price, search | Free tier with unlimited connected accounts, friends and groups and **limited chat usage**; Pro unlimited chat | Verified | Unlimited search, free, offline |
| Price, Pro | **$24/person/month billed annually**; indexed pricing description also lists **$30 monthly** | Verified | $0 local; indicative $8/mo optional cloud ([pricing-and-cloud.md](pricing-and-cloud.md)) |
| Free search allowance | "about 15 searches/month" | **Unconfirmed** (third-party; not on current pricing page) | n/a |
| Export | CSV export is a Pro feature | Verified | The SQLite file is the format; `openstance export --format csv\|json\|vcf\|parquet` always free |
| API / MCP | Network search, person research, group access and credit tools via MCP; search and research run asynchronously; separate credits | Verified | MCP server, HTTP API and CLI in the same binary, free; synchronous |
| Credit costs | 2 credits per search, 1 per person research, no public $/credit | Overview | n/a |
| Results per search | Up to 30, async with polling, "find more" for the next page | Overview | All matches, synchronous, paginated locally |
| Relationship ranking | Ranked by strength; weighting not documented publicly | Verified that ranking exists; logic not found published | Five visible components (R, F, C, M, B), weights in TOML, presets ([../tech-spec/scoring.md](../tech-spec/scoring.md)) |
| Match explanation | Per-person reasons; vendor listing says it attempts to provide concrete evidence and acknowledges LLM responses can be inaccurate | Verified (vendor Slack listing) | Deterministic evidence lines (thread subjects, meeting titles, dates, matched fields); prose optional ([../tech-spec/search.md](../tech-spec/search.md)) |
| Accuracy reports | One independent review reports a misclassified role in its own test | Single report, not representative | Extracted filters shown and editable; rerun without a model; field overrides |
| Correcting a profile | Not established. The review above found no obvious correction path; current capability not verified | Unconfirmed | Every field editable; overrides survive re-import, rebuild and enrichment ([../tech-spec/data-model.md](../tech-spec/data-model.md)) |
| Outside data | Web search inside the chat, included in metered usage | Overview | Off by default; per call via an `Enricher` (none, treg, own keys); price quoted first; capped and logged ([../tech-spec/enrichment.md](../tech-spec/enrichment.md)) |
| Groups / teams | Free groups pooling members' networks; communities page shows drafted intro requests | Verified | None in v1. Phase 5: federated views over git/S3, no central database |
| Scale / network effect | Overview cites 500k+ users and groups of 380k+ people | Overview (vendor-stated, not re-checked) | None; distribution via OSS, agents and the free-search story |
| Mobile | iPhone app | Verified | None in v1 |
| Models | Vendor's own, not configurable | Verified by absence of configuration in docs | User's choice: none, Ollama, OpenAI-compatible, Anthropic ([../tech-spec/models.md](../tech-spec/models.md)) |

### Where Happenstance still wins

Be explicit about this in every comparison:

- **Breadth of live connectors.** Seven OAuth sources that stay fresh without the user re-exporting anything. openstance requires periodic exports in v1.
- **Extended-network and group reach.** Pooled community groups give access to people far outside the user's own graph. A local file cannot do this, and openstance does not try in v1.
- **Mobile.** An iPhone app; openstance has none.
- **Zero setup.** Sign in and connect accounts, versus download exports and run a binary.
- **Web research in the loop.** Agentic search over the web for people not in your network, without configuring an enricher.
- **Maturity.** An operating company with security reviews and support.

### Where openstance should win (to be measured)

| Claim | How we will test it |
|---|---|
| Search costs nothing and has no limit | True by construction; no metering code exists |
| Faster first answer after import | Time-to-first-shortlist in pilot; seconds after import is the target ([../engineering/performance-targets.md](../engineering/performance-targets.md)) |
| Fewer unsupported matches | Search eval: zero unsupported facts in evidence lines; manual review of failures |
| User can fix wrong data and see it stick | `edit` then `rebuild`; property test that rebuild preserves overrides |
| Data stays under the user's control | Only egress path is the `Enricher`; every call in `enrich_log` with a prior quote |
| Free agent access | `openstance mcp` in the same binary |

## Adjacent products

Prices for adjacent products are not verified in our research. The overview groups "Dex, Village and The Swarm" as charging roughly $12 to $99 per month for follow-through features; treat that range as indicative and re-check vendor pages before quoting it.

| Product | What it is (sourced where noted) | Overlap with openstance | Where it still wins | Our position |
|---|---|---|---|---|
| **Monica** | Open-source, self-hostable personal relationship manager with notes, relationships, activities and reminders ([monicahq.com](https://www.monicahq.com/en/), verified) | Open source, user-owned contact data | Broad personal memory (family, gifts, life events), mature self-host story, polished UI | Open source and ownership are **not unique** to us. We focus on professional discovery, relationship scoring from communication metadata, paths and agent access, not broad personal memory |
| **Mesh** (the destination now reached from clay.earth; formerly Clay) | Unified contacts, search, updates and reconnect reminders ([me.sh](https://me.sh/), verified) | Contact unification, search, reconnect | Automatic sync, polished consumer UX, mobile | Reminders and relationship memory are established territory. Our `reconnect` playbook is free and local but not a reason on its own to switch |
| **The Swarm** | Relationship mapping, people and company data, enrichment, team network pooling, API/MCP integrations ([theswarm.com](https://www.theswarm.com/), verified) | Team relationship mapping, account mapping, MCP | A public people/company data operation; team pooling; sales-team workflows | Building a comparable data operation is a different business. We map coverage from the user's own graph (`accounts`) and pull outside data only per call |
| **Dex** | Personal CRM with contact sync and keep-in-touch reminders (overview; not re-checked) | Reconnect, notes, LinkedIn-derived contacts | Live LinkedIn/contact sync, reminders delivered to you, mobile | Our reconnect digest and job-change watch are local and free; scheduled delivery is a cloud-tier feature ([pricing-and-cloud.md](pricing-and-cloud.md)) |
| **Village** | Warm-introduction paths across colleagues' and community networks (overview; not re-checked) | Path to a stranger, intro requests | Pooled organizational networks; intro request workflow | We compute paths only over the user's graph plus explicitly shared views; no pooled database |
| **Contrack** | Described in the overview as the closest prior attempt at a local, file-based contact graph, without an agent story (not re-checked) | Local file, ownership | Unknown; verify before citing | Verify existence, license and current status before referencing publicly |
| **Boomerang** | Cited in the overview as where intro orchestration happens for some Happenstance users (not re-checked) | Follow-through after the search | Email workflow tooling | We generate briefs; we do not send email |

## Positioning statement

For founders, job seekers, recruiters, sellers and community managers who live on warm introductions and are uneasy putting their whole relationship graph in someone else's cloud, **openstance** is a local-first people-search engine that turns the exports they already have into one SQLite file they can search for free from the terminal or any agent. Unlike hosted network-search services, the graph never leaves the machine unless the user exports a slice or approves a quoted lookup, every result shows the facts behind it, and every field can be corrected.

## Risks in this analysis

| Risk | Mitigation |
|---|---|
| Vendor pricing and features change monthly | Date every claim; re-verify before launch ([../plan/launch.md](../plan/launch.md) checklist) |
| Comparison reads as parody or invites a trademark complaint | Ground rules above; name decision in ADR-0001 |
| We overstate differentiation | "Where Happenstance still wins" section is mandatory in any public comparison |
| No user interviews or hands-on paid-account comparison yet | Pilot in Phase 1/2 compares the user's current method, Happenstance where participants have access, and openstance on the same questions; record whether the target person was in the imported data |

Tracked in [../risks-and-open-questions.md](../risks-and-open-questions.md).

## Related documents

- [vision.md](vision.md), [personas-and-use-cases.md](personas-and-use-cases.md), [pricing-and-cloud.md](pricing-and-cloud.md)
- [../prototype-v0.1.md](../prototype-v0.1.md) (where the research came from)
