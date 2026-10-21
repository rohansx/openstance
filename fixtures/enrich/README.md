# Enrichment fixtures

Canned responses for `MockEnricher` (`openstance-enrich`, feature `mock` or
`test-util`). Every `*.json` file in this directory holds one entry or an
array of entries:

```json
{ "request": <EnrichRequest>, "data": <EnrichData>, "provider": "mockdata", "cost_micro_usd": 10000 }
```

`provider` and `cost_micro_usd` are optional (the cost defaults to the mock's
quoted price for that op). A request with no entry returns `not_found`.

`treg/` holds treg-shaped HTTP bodies used by the `TregEnricher` tests. They
were written by hand for openstance from treg's documented response contract;
no treg code or catalog data is copied.

All people, addresses and URLs here are made up.
