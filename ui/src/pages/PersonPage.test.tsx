import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { fx } from '../test/fixtures';
import { renderApp } from '../test/render';

const EDITS = { 'POST /edits': { applied: 1, people: [{ id: 812, display_name: 'Tom Okafor', title: 'CTO', company: 'Stripe', score: 0.78 }] } };

function lastOps(api: ReturnType<typeof renderApp>['api']) {
  return (api.callsTo('POST /edits').at(-1)!.body as { ops: unknown[] }).ops;
}

async function openTom(routes: Record<string, unknown> = {}) {
  const r = renderApp('#/people/812', { ...EDITS, ...routes });
  await screen.findByRole('heading', { level: 1, name: 'Tom Okafor' });
  return r;
}

const row = (label: string) => screen.getByRole('group', { name: label });

describe('Person page (P4-UI-03)', () => {
  it('shows each field with where it came from', async () => {
    await openTom();
    expect(within(row('Title')).getByText('Engineering Manager')).toBeInTheDocument();
    expect(within(row('Title')).getByText(/your edit/)).toBeInTheDocument();
    expect(within(row('Company')).getByText(/linkedin 2026-10 \(linkedin_csv\)/)).toBeInTheDocument();
    expect(within(row('Location')).getByText(/treg \(enrich\)/)).toBeInTheDocument();
    expect(within(row('Bio')).getByText(/unknown/i)).toBeInTheDocument();
  });

  it('inline edit writes a set override and reloads the person', async () => {
    const { api } = await openTom();
    await userEvent.click(within(row('Title')).getByRole('button', { name: /edit title/i }));
    const input = within(row('Title')).getByRole('textbox', { name: /title/i });
    await userEvent.clear(input);
    await userEvent.type(input, 'CTO');
    await userEvent.click(within(row('Title')).getByRole('button', { name: /^save$/i }));

    await waitFor(() => expect(api.callsTo('POST /edits')).toHaveLength(1));
    expect(lastOps(api)).toEqual([{ op: 'set', person: 812, field: 'title', value: 'CTO' }]);
    await waitFor(() => expect(api.callsTo('GET /people/812').length).toBeGreaterThanOrEqual(2));
  });

  it('clearing a field writes an unset tombstone', async () => {
    const { api } = await openTom();
    await userEvent.click(within(row('Location')).getByRole('button', { name: /clear location/i }));
    await waitFor(() => expect(api.callsTo('POST /edits')).toHaveLength(1));
    expect(lastOps(api)).toEqual([{ op: 'unset', person: 812, field: 'location' }]);
  });

  it('accepting an enrichment suggestion is an explicit set', async () => {
    const { api } = await openTom();
    expect(within(row('Title')).getByText(/Sr\. EM/)).toBeInTheDocument();
    await userEvent.click(within(row('Title')).getByRole('button', { name: /use suggested title/i }));
    await waitFor(() => expect(lastOps(api)).toEqual([{ op: 'set', person: 812, field: 'title', value: 'Sr. EM' }]));
  });

  it('tags and notes', async () => {
    const { api } = await openTom();
    const tags = screen.getByRole('region', { name: 'Tags and notes' });
    await userEvent.click(within(tags).getByRole('button', { name: /remove tag devtools/i }));
    await waitFor(() => expect(lastOps(api)).toEqual([{ op: 'untag', person: 812, value: 'devtools' }]));

    await userEvent.type(within(tags).getByRole('textbox', { name: /new tag/i }), 'investor');
    await userEvent.click(within(tags).getByRole('button', { name: /add tag/i }));
    await waitFor(() => expect(lastOps(api)).toEqual([{ op: 'tag', person: 812, value: 'investor' }]));

    expect(within(tags).getByText('prefers short asks')).toBeInTheDocument();
    await userEvent.type(within(tags).getByRole('textbox', { name: /new note/i }), 'met at a demo day');
    await userEvent.click(within(tags).getByRole('button', { name: /add note/i }));
    await waitFor(() => expect(lastOps(api)).toEqual([{ op: 'note', person: 812, value: 'met at a demo day' }]));
  });

  it('do_not_share and no_enrich toggles write flag overrides', async () => {
    const { api } = await openTom();
    await userEvent.click(screen.getByRole('checkbox', { name: /do not share/i }));
    await waitFor(() => expect(lastOps(api)).toEqual([{ op: 'flag', person: 812, field: 'do_not_share', value: true }]));
    await userEvent.click(screen.getByRole('checkbox', { name: /do not enrich/i }));
    await waitFor(() => expect(lastOps(api)).toEqual([{ op: 'flag', person: 812, field: 'no_enrich', value: true }]));
  });

  it('lists identities with sources and splits selected identities off', async () => {
    const { api } = await openTom();
    const ids = screen.getByRole('region', { name: 'Identities' });
    expect(within(ids).getByText('tom@stripe.example')).toBeInTheDocument();
    expect(within(ids).getByText(/deliverable/)).toBeInTheDocument();
    expect(within(ids).getAllByText(/work gmail/).length).toBeGreaterThan(0);
    await userEvent.click(within(ids).getByRole('checkbox', { name: /select in\/tomokafor-example/i }));
    await userEvent.click(within(ids).getByRole('button', { name: /split 1 identity off/i }));
    await waitFor(() => expect(lastOps(api)).toEqual([{ op: 'split', person: 812, identities: [5600] }]));
  });

  it('shows interactions, mutuals, score components and the how-connected panel', async () => {
    const { api } = await openTom();
    expect(screen.getByText('2026-09-14 · meeting mutual · "devtools dinner" · work calendar')).toBeInTheDocument();
    const mutuals = screen.getByRole('region', { name: 'Mutuals' });
    expect(within(mutuals).getByRole('link', { name: 'Ana Lee' })).toHaveAttribute('href', '#/people/77');
    expect(within(mutuals).getByText(/9 shared email/)).toBeInTheDocument();

    const score = screen.getByRole('region', { name: 'Relationship score' });
    expect(within(score).getByText('0.870')).toBeInTheDocument();
    expect(within(score).getByText('× 0.35')).toBeInTheDocument();
    expect(within(score).getByText(/23 interactions/)).toBeInTheDocument();

    const how = screen.getByRole('region', { name: 'How connected' });
    expect(await within(how).findByRole('link', { name: 'Tom Okafor' })).toBeInTheDocument();
    expect(within(how).getByText('0.406')).toBeInTheDocument();
    expect(api.callsTo('GET /connections')[0]!.url.searchParams.get('target')).toBe('812');
  });

  it('tells the user when the id was merged into another person', async () => {
    renderApp('#/people/97', { 'GET /people/97': { ...fx.person(), resolved_from: 97 } });
    expect(await screen.findByText(/#97 was merged into this person/i)).toBeInTheDocument();
  });

  it('shows a not-found error', async () => {
    renderApp('#/people/5', { 'GET /people/5': { status: 404, body: { v: 1, error: { code: 'not_found', message: 'no person 5' } } } });
    expect(await screen.findByRole('alert')).toHaveTextContent('no person 5');
  });
});
