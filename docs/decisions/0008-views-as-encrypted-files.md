# ADR-0008: Share views as age-encrypted files, never the graph

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/sharing-and-views.md](../tech-spec/sharing-and-views.md), [architecture/security-and-threat-model.md](../architecture/security-and-threat-model.md), [plan/phase-3.md](../plan/phase-3.md)

## Context

Founders want to share part of their network ("my devtools investors") with a co-founder or a community, without handing over every email header they have. Incumbent models are "friends see your full network" or "groups pool everyone's accounts in one cloud". The overview's line is: share slices, never the graph.

Constraints: no openstance server in v1; the recipient may be offline; revocation of a file someone already has is impossible in general.

## Decision

- A **view** is a saved query (`view` table: `id, name, query_json, created_at, expires_at, recipient_pubkey`).
- `openstance view export <name>` materializes the query at export time into a small, self-contained SQLite file and **encrypts it with age** to the recipient's X25519 public key. File extension `.ost`.
- File contents:
  - `person`: only the rows the query returns, **minus `do_not_share` people**, with a score bucket (not the raw components) and last-contact month (not the date).
  - `identity`: emails and LinkedIn URLs only.
  - `meta`: format version, owner pubkey, created, expires, `query_json`, view id.
  - **No interactions** (no email subjects, meeting titles or DM metadata).
- Keys are generated at `openstance init` (`~/.openstance/age.key`); recipients share their public key once.
- `openstance view import <file>` refuses files past `expires_at`. Beyond that, expiry is advisory.
- Revocation = do not re-export. This is documented as the honest limit of an offline file.
- Recipients query imported views with `search_view` (MCP) / `openstance search --view <name>`; imported views never merge into the recipient's own graph.
- Phase 5: `openstance sync` pushes and pulls encrypted views over a git remote or S3-compatible bucket; team mode is the same with a shared remote and query-time federation.

## Consequences

**Positive**

- Nothing is shared by default; the unit of sharing is explicit and reviewable before export.
- No central database holding everyone's contacts; works without any openstance server.
- age is small, audited and has a stable Rust implementation.

**Negative**

- TTL and revocation cannot be enforced against a recipient who keeps the decrypted file. Stated plainly in the CLI output and docs.
- Shared data is a point-in-time snapshot; recipients must ask for a re-export to refresh.
- Federated team search is weaker in reach than pooled groups (accepted; see [product/competitive-analysis.md](../product/competitive-analysis.md)).

## Alternatives considered

| Option | Why not |
|---|---|
| Shared hosted graph | Requires a server and a central store of everyone's contacts; contradicts the product |
| Plain CSV export | No encryption, no expiry metadata, leaks to anyone who sees the file |
| GPG | Heavier key management; age is simpler for one-time public-key exchange |
| SQLCipher with a shared passphrase | Passphrase distribution is worse than public keys |
