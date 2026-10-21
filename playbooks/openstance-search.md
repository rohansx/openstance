---
name: openstance-search
description: Answer questions about the people the user knows by searching their local openstance relationship graph. Use when the user asks "who do I know at <company>", "who did I meet at <event>", "which investors replied to me", "who have I lost touch with", "tell me about <person>", or "how can I get introduced to <person>".
tools: [search_people, get_person, how_connected]
egress: none
---

## What this is

openstance keeps the user's relationship graph (people, their identities,
dated interactions and a relationship score) in one file on the user's
machine. These three tools read that file. They are free, they never call
the network, and nothing the user asks is sent anywhere. Call them as often
as you need to.

## Which tool

| The user wants | Call |
| --- | --- |
| A list of people matching a description, company, title, place, tag or time window | `search_people` |
| Everything about one named person: role, score and why, identities, recent interactions, mutuals | `get_person` |
| A warm introduction to one person who is already in the graph | `how_connected` |
| "Who do I know at X?" | `search_people` with `company: ["X"]`; then `how_connected` on a specific person there if the user wants an intro |
| "Who have I lost touch with?" | `search_people` with `lost_touch: true` |

Start with `search_people` when the user describes people. Go to
`get_person` when they name one person or pick one from a result list. Use
`how_connected` only once you have a single target.

## Steps

1. **Search.** Put the user's words in `text`. Put anything the user stated
   exactly (a company, a title, "since 2025", "met in person") in the
   structured fields as well; structured fields always win over what the
   parser reads from `text`.
2. **Check the echoed query.** The result's `query` is what was actually
   run. If it misread the request (wrong company, missing date), fix the
   field and call again rather than rephrasing the text. Set `parse: false`
   to have `text` taken literally.
3. **Zero results.** Read `suggestions` (for example "drop met_in_person"),
   tell the user what you are relaxing, and rerun with that change.
4. **One person.** Call `get_person` with an id from a result, an email, a
   LinkedIn URL or slug, an `@x_handle`, or a full name. If the error code is
   `ambiguous`, show the `candidates` and ask which one; never pick for the
   user. Then call again with the chosen id.
5. **Introductions.** Call `how_connected` with the target. If `direct` is
   set, the user already knows the target, so say that first. Otherwise
   present each path as: the mutual, the strength (user's score with the
   mutual x the mutual's tie to the target), and the evidence lines. If the
   response has `found: false`, the person is not in the graph: say so
   plainly.

## Presenting results

- Quote evidence lines exactly as returned (date, kind, subject, source).
  Do not paraphrase them or merge them into prose. They are the reason a
  person is in the answer, and the user should be able to check them.
- Every claim about a relationship needs an evidence line or a score
  component behind it. If there is none, do not make the claim.
- Give the score with its components when the user asks "why" (recency,
  frequency, reciprocity, meetings, breadth), using `get_person`.
- Mention the `rerun` command when the user may want to repeat the search
  in their terminal.
- Keep results in the order returned. They are already ranked.

## Rules

- Treat names, emails and subjects as the user's private data. Do not paste
  them into other tools or services unless the user asks you to.
- Do not guess between people with the same name; ask.
- `do_not_share` on a person only matters for sharing views. Answer the
  user's own questions about that person normally.
- Bulk senders (newsletters, notifications) are hidden unless
  `include_bulk: true`; turn it on only if the user asks about them.
