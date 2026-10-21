---
name: openstance-path
description: Find a warm introduction path to one specific person using the user's local openstance graph, including people the user has never been in touch with. Use when the user asks "how can I get to X", "who can introduce me to X", or "do I know anyone who knows X at <company>".
tools: [how_connected, path_to]
egress: optional (profile, find_email, verify_email)
---

## When to use

- The user names one target person, optionally with their company or a
  LinkedIn link.
- Not for "who do I know at <company>". That is a search: use
  `search_people` with a company filter instead.

## What it costs

Everything about the user's own graph is local and free. When the target
is not in the graph, openstance can look up the target's public profile
(current and past employers, school) and, if asked, find and check their
work email. Those lookups are paid requests to the enrichment backend the
user configured. They cost cents, and they are never made unless the call
explicitly allows egress. A call that does not returns a price (a quote)
and sends nothing.

## Steps

1. Call `how_connected` with the target. If the target is in the graph,
   present the top paths with their evidence lines and stop. No paid step
   is needed.
2. Otherwise call `path_to` with `allow_egress: false` and the company if
   the user gave one. Add `find_email: true` only if the user asked for an
   email address. This call is free and sends nothing.
3. Read the result:
   - `status: "quote"`: show the user the quote in plain words. Give the
     total (`plan.quote.total_micro_usd`, in millionths of a dollar), the
     number of requests, and exactly which fields would be sent (for
     example the target's name and company). Also show any free paths the
     local steps already found in `result.paths`.
   - `status: "done"` or `"nothing_to_do"`: there is nothing to pay for.
     Present the result.
4. Ask the user whether to spend that amount. Only after the user
   explicitly agrees in this conversation, call `path_to` again with the
   same arguments plus `allow_egress: true`. Pass `max_usd` if the user
   named a ceiling; it can only lower the configured budget.
5. Present each path as: the mutual, the strength, why they connect
   (`same_company`, `past_company`, `school` or `tie`) and the dated
   evidence lines exactly as returned. Mention what the run cost
   (`receipt.spent_micro_usd`).

## Rules

- Never set `allow_egress: true` without the user's explicit consent for
  this quote. Agreement to an earlier quote does not carry over to a new
  target or a bigger quote.
- Never retry a failed paid call more than once. If the note says the
  backend refused (no credit, bad token, budget), tell the user and stop.
- If `plan.quote.skipped` lists a person as `no_enrich` or
  `do_not_share`, tell the user that person is excluded from lookups. Do
  not work around it with other tools.
- A path is a suggestion, not a fact about the target. Never invent a
  relationship that has no evidence line, and say when a match is only a
  shared past employer or school.
- Do not paste the target's email address or the user's contacts into
  other tools or services unless the user asks you to.
