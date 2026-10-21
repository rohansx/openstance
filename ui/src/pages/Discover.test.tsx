import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { SearchResponse } from '../api/types';
import { fx } from '../test/fixtures';
import { renderApp } from '../test/render';

function withQuery(patch: Partial<SearchResponse['query']>, extra: Partial<SearchResponse> = {}): SearchResponse {
  const r = fx.search();
  return { ...r, ...extra, query: { ...r.query, ...patch } };
}

async function search(text: string) {
  const box = await screen.findByRole('searchbox', { name: /search your network/i });
  await userEvent.clear(box);
  await userEvent.type(box, text);
  await userEvent.click(screen.getByRole('button', { name: /find people/i }));
}

describe('Search page (P4-UI-02)', () => {
  it('sends the typed text to the parser and renders result cards with score bar and evidence', async () => {
    const { api } = renderApp('#/discover');
    await search('devtools dinner');

    await waitFor(() => expect(api.callsTo('POST /search').some((c) => (c.body as { text?: string }).text === 'devtools dinner')).toBe(true));
    const call = api.callsTo('POST /search').find((c) => (c.body as { text?: string }).text === 'devtools dinner')!;
    expect(call.body).toMatchObject({ text: 'devtools dinner', parse: true });

    const card = (await screen.findByRole('heading', { name: 'Tom Okafor' })).closest('article')!;
    expect(within(card).getByRole('meter', { name: /relationship score/i })).toHaveAttribute('aria-valuenow', '78');
    expect(within(card).getByText('2026-09-14 · meeting mutual · "devtools dinner" · work calendar')).toBeInTheDocument();
    expect(within(card).getByText(/Engineering Manager · Stripe/)).toBeInTheDocument();
    expect(within(card).getByRole('link', { name: /open tom okafor/i })).toHaveAttribute('href', '#/people/812');

    const kai = (await screen.findByRole('heading', { name: 'Kai Wu' })).closest('article')!;
    expect(within(kai).getByText(/do not share/i)).toBeInTheDocument();
    expect(within(kai).getByText(/no interactions in evidence/i)).toBeInTheDocument();
    expect(screen.getByText(/2 people/)).toBeInTheDocument();
  });

  it('shows the parsed filters as chips and removing one reruns without the parser', async () => {
    const { api } = renderApp('#/discover');
    await search('devtools dinner');
    const chips = await screen.findByRole('group', { name: /parsed filters/i });
    expect(within(chips).getByText('met in person')).toBeInTheDocument();
    const before = api.callsTo('POST /search').length;

    await userEvent.click(within(chips).getByRole('button', { name: /remove filter met in person/i }));

    await waitFor(() => expect(api.callsTo('POST /search').length).toBe(before + 1));
    const rerun = api.callsTo('POST /search').at(-1)!.body as Record<string, unknown>;
    expect(rerun.parse).toBe(false);
    expect(rerun.text).toBe('devtools dinner');
    expect(rerun).not.toHaveProperty('met_in_person');
  });

  it('edits a chip value and reruns with parse:false', async () => {
    const { api } = renderApp('#/discover', { 'POST /search': withQuery({ company: ['Stripe'] }) });
    await search('people at stripe');
    const chips = await screen.findByRole('group', { name: /parsed filters/i });
    await userEvent.click(within(chips).getByRole('button', { name: /edit filter company: stripe/i }));
    const input = within(chips).getByRole('textbox', { name: /company/i });
    await userEvent.clear(input);
    await userEvent.type(input, 'Ramp{Enter}');

    await waitFor(() => expect((api.callsTo('POST /search').at(-1)!.body as Record<string, unknown>).company).toEqual(['Ramp']));
    expect(api.callsTo('POST /search').at(-1)!.body).toMatchObject({ parse: false, company: ['Ramp'] });
  });

  it('adds a filter chip and reruns without the parser', async () => {
    const { api } = renderApp('#/discover');
    await search('devtools dinner');
    await screen.findByRole('group', { name: /parsed filters/i });
    await userEvent.selectOptions(screen.getByRole('combobox', { name: /filter field/i }), 'title');
    await userEvent.type(screen.getByRole('textbox', { name: /filter value/i }), 'founder');
    await userEvent.click(screen.getByRole('button', { name: /add filter/i }));
    await waitFor(() => expect(api.callsTo('POST /search').at(-1)!.body).toMatchObject({ parse: false, title: ['founder'], met_in_person: true }));
  });

  it('shows warnings, suggestions, the parser and the CLI rerun line', async () => {
    renderApp('#/discover', {
      'POST /search': withQuery({}, {
        warnings: ['embedder unavailable: text search only'],
        suggestions: ['drop met_in_person -> 12 results'],
        query_source: { parser: 'ollama:qwen2.5:7b', fallback: 'timeout', explicit: [] },
      }),
    });
    await search('devtools dinner');
    expect(await screen.findByText('embedder unavailable: text search only')).toBeInTheDocument();
    expect(screen.getByText('drop met_in_person -> 12 results')).toBeInTheDocument();
    expect(screen.getByText(/parser: ollama:qwen2\.5:7b/)).toBeInTheDocument();
    expect(screen.getByText(/fell back: timeout/)).toBeInTheDocument();
    expect(screen.getByText(/openstance search "devtools dinner" --met --no-parse/)).toBeInTheDocument();
  });

  it('toggles the shortlist from a result card', async () => {
    const { api } = renderApp('#/discover', { 'DELETE /shortlist/812': { status: 204 }, 'POST /shortlist': { person_id: 1203, note: null } });
    await search('devtools dinner');
    const tom = (await screen.findByRole('heading', { name: 'Tom Okafor' })).closest('article')!;
    const save = await within(tom).findByRole('button', { name: /remove tom okafor from shortlist/i });
    expect(save).toHaveAttribute('aria-pressed', 'true');
    await userEvent.click(save);
    await waitFor(() => expect(api.callsTo('DELETE /shortlist/812')).toHaveLength(1));

    const kai = screen.getByRole('heading', { name: 'Kai Wu' }).closest('article')!;
    await userEvent.click(within(kai).getByRole('button', { name: /save kai wu to shortlist/i }));
    await waitFor(() => expect(api.callsTo('POST /shortlist')[0]?.body).toEqual({ person_id: 1203 }));
  });

  it('People page lists everyone by score with a structured (no text) search', async () => {
    const { api } = renderApp('#/people');
    await screen.findByRole('heading', { name: 'Tom Okafor' });
    const first = api.callsTo('POST /search')[0]!.body as Record<string, unknown>;
    expect(first).not.toHaveProperty('text');
    expect(first.parse).toBe(false);
  });
});
