# ADR-0001: Product name

- **Status:** Proposed (open)
- **Date:** 2026-10-20
- **Deciders:** Rohan (maintainer)
- **Related:** [risks-and-open-questions.md](../risks-and-open-questions.md), [product/competitive-analysis.md](../product/competitive-analysis.md), [plan/launch.md](../plan/launch.md)

## Context

The project needs a name for the repository, the binary, the crates, the data directory, the share-file extension and the launch post. The name is load-bearing in several places at once:

| Where the name appears | Current value |
|---|---|
| Binary / short alias | `openstance` / `ost` |
| Crates | `openstance-core`, `openstance-import`, `openstance-enrich`, `openstance-model`, `openstance-mcp`, `openstance-api`, `openstance` |
| Data directory | `~/.openstance/` (`graph.db`, `config.toml`, `age.key`) |
| View file extension | `*.ost` |
| Local HTTP API | `127.0.0.1:7411` (name-independent) |

The v0.1 prototype and the tech spec both use "openstance". The product overview recommends **kith** instead, because a name built on "stance" reads as "open Happenstance" to the people who know the incumbent, reads as "open stance" (an opinion) to everyone else, and invites a trademark complaint. The prototype README already notes: "The Openstance name has not been cleared for commercial use."

The overview's own options table, reproduced and extended:

| Name | For | Against |
|---|---|---|
| **openstance** | Signals "open Happenstance" to people who know the target; already used in prototype, spec and this doc set | Reads as "open stance"; leans on a competitor's mark; trademark-letter risk; looks like a parody name |
| **kith** | Means "the people you know" (kith and kin); one syllable; short CLI; tagline writes itself ("the people you know, as a file") | Obscure to non-native speakers; `kith.sh` / `kith.dev` / crates.io availability unverified |
| rolodex / rolo | Instantly understood | Generic; many existing apps; weak CLI name; "Rolodex" is itself a trademark |
| whoknows | Describes the core query ("who knows someone at Stripe") | Hard to brand and to search for |
| warmth | Names the score (warm intros) | Abstract; no CLI feel |
| mutuals | Describes the path feature | Plural noun; social-app connotation |
| graphd / peoplegraph | Technical and honest | Boring; already used by several projects |

## Decision

**Not yet decided.** Until this ADR is accepted, "openstance" is the working name used everywhere (code, docs, config paths), and no other doc uses "kith" as the product name.

The decision must be made **before the first public release (Phase 2 Show HN)**, because after that the crate names, data directory and file extension become a compatibility surface. See [plan/phase-2.md](../plan/phase-2.md).

Leaning: kith, provided the checks below pass.

### Checks required to accept

1. crates.io: `kith`, `kith-core` (or the chosen prefix) are unclaimed, or the owner will transfer them.
2. Domains: `kith.sh` or `kith.dev` (or an acceptable alternative) is registrable.
3. GitHub org or repo name is available.
4. A basic trademark search (USPTO TESS, EUIPO) in software classes (9, 42) shows no conflicting live mark.
5. Homebrew formula name and the short alias do not collide with an existing common CLI.

## Consequences

If **kith** is accepted:

- Rename is mechanical while the codebase is small: crate prefix `kith-*`, binary `kith`, data dir `~/.kith/`, view extension to be chosen (for example `.kith`). Do it in one commit with a migration note in [tech-spec/configuration.md](../tech-spec/configuration.md). Phase 1 code should keep the name in a single constant (`APP_NAME`, `DATA_DIR`, `VIEW_EXT`) to make this cheap.
- Docs are updated by search and replace plus a review of prose that relies on "open" in the name.
- The short alias `ost` is dropped (a one-word name does not need one).

If **openstance** is kept:

- Competitive copy must stay strictly factual (see [product/competitive-analysis.md](../product/competitive-analysis.md)); no parody branding, no use of the Happenstance name or styling in the logo or tagline.
- Accept residual trademark risk and keep a rename plan ready.

Either way:

- The name is recorded in exactly one place in code, so later renames are cheap.
- View files carry a format magic and version in their `meta` table, independent of the file extension, so a rename does not break import of older views.

## Alternatives considered

See the table above. "rolodex", "whoknows", "warmth", "mutuals" and "graphd" were all rejected in the overview for branding or collision reasons; they are listed so the decision record is complete.
