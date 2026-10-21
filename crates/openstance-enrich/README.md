# openstance-enrich

The enrichment seam: the `Enricher` trait, its request / quote / result types,
and the backends. This is the only crate that may send data about the user's
contacts off the machine. Budgets, `no_enrich`, `do_not_share` and
`enrich_log` are enforced in `openstance-core` (the egress gate), never here.
Spec: [`docs/tech-spec/enrichment.md`](../../docs/tech-spec/enrichment.md).

| Backend | Feature | `enrich.backend` | Notes |
| --- | --- | --- | --- |
| `NullEnricher` | always | `none` (default) | supports nothing; `enrich` returns "enrichment is off: set enrich.backend" |
| `treg::TregEnricher` | `treg` | `treg` | blocking HTTP (ureq) to a treg router; token from `OPENSTANCE_TREG_TOKEN` |
| `ownkeys::OwnKeysEnricher` | `ownkeys` | `ownkeys` | reserved stub, see below |
| `mock::MockEnricher` | `mock` / `test-util` | tests only | records calls, answers from `fixtures/enrich/*.json` |

The trait is sync. Backends never pull an async runtime into core.

## OwnKeys (planned, unscheduled)

`OwnKeysEnricher` exists so `enrich.backend = "ownkeys"` is a reserved value.
Today its `quote` and `enrich` return `Unsupported`. The planned providers,
with keys read from the environment only:

| Provider | Env var | Ops |
| --- | --- | --- |
| People Data Labs | `PDL_API_KEY` | profile, company |
| Apollo | `APOLLO_API_KEY` | profile, find_email, company |
| Clearbit (now part of HubSpot; confirm availability) | `CLEARBIT_API_KEY` | company |
| Any email verifier with a simple GET API | `[enrich.ownkeys.verify]` | verify_email |

Quotes will always be `basis = estimate`, from `[enrich.ownkeys.prices]`.

## Tests

```sh
cargo test -p openstance-enrich --all-features
```

The treg tests run against an in-process mock HTTP server on 127.0.0.1 with
hand-written, treg-shaped bodies in `fixtures/enrich/treg/`. Nothing reaches
the internet.
