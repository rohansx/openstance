---
name: openstance-brief
description: Prepare a short, fact-only brief about one person in the user's openstance graph before a meeting, a call or an introduction request. Use when the user asks "brief me on X", "remind me how I know X", "what should I say to X", or wants context before asking someone for an intro.
tools: [get_person, brief]
egress: optional (web_search)
---

## When to use

- The user names one person who is already in their graph and wants a
  quick summary: who they are, how the user knows them, when they last
  spoke, the best mutual connection and a suggested opener.
- For a full record of the person (every identity, score components, all
  recent interactions), use `get_person` instead.

## What it costs

The brief is built from the user's own graph and is free. One optional
step, `web: true`, runs a single paid web search for the person's recent
public posts. That search is only sent when the call explicitly
allows egress; otherwise the call returns the brief from local data plus a
quote for the search.

## Steps

1. If the name could match several people, call `get_person` first. On an
   `ambiguous` error, show the candidates and ask which one; then use the
   id.
2. Call `brief` with the person and `allow_egress: false`. Leave `web`
   off unless the user asked for recent public activity.
3. If you set `web: true` and the result has `status: "quote"`, show the
   user the price of the search (`plan.quote.total_micro_usd`, in
   millionths of a dollar) and the query that would be sent. The local
   brief is already in `result.markdown`; you can share it right away.
4. Only after the user explicitly agrees, call `brief` again with the same
   arguments plus `allow_egress: true`.
5. Show `result.markdown` as returned. It is at most ten lines, in a fixed
   order, and every line is backed by an entry in `result.facts`
   (interaction ids or enrichment log ids). Do not add lines of your own
   to it; put any extra advice after it, clearly marked as yours.

## Rules

- Never set `allow_egress: true` without the user's explicit consent for
  that quote.
- If the notes say the person is `no_enrich`, the web search is skipped on
  purpose. Tell the user; do not search for them with other tools.
- The suggested opener only uses facts from the brief. Keep it that way if
  you rephrase it.
- Treat everything in the brief as the user's private data. Do not paste
  it into other tools or services unless the user asks you to.
