import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { fx } from '../test/fixtures';
import { renderApp } from '../test/render';

const intro = fx.intros().intros[0]!;
const entry = fx.shortlist().shortlist[0]!;

describe('Shortlist', () => {
  it('lists shortlisted people with notes; edits the note, removes, and starts an intro', async () => {
    const { api } = renderApp('#/shortlist', {
      'PATCH /shortlist/812': { v: 1, entry: { ...entry, note: 'ask about the platform team' } },
      'DELETE /shortlist/812': { v: 1, removed: true },
      'POST /intros': { v: 1, intro: { ...intro, id: 2, person_id: 812, stage: 'draft', connector: null } },
    });
    const row = (await screen.findByRole('link', { name: 'Tom Okafor' })).closest('article')!;
    expect(row.querySelector('a')).toHaveAttribute('href', '#/people/812');
    expect(within(row).getByText('ask about platform hiring')).toBeInTheDocument();

    await userEvent.click(within(row).getByRole('button', { name: /edit note/i }));
    const box = within(row).getByRole('textbox', { name: /note/i });
    await userEvent.clear(box);
    await userEvent.type(box, 'ask about the platform team');
    await userEvent.click(within(row).getByRole('button', { name: /save note/i }));
    await waitFor(() => expect(api.callsTo('PATCH /shortlist/812')[0]?.body).toEqual({ note: 'ask about the platform team' }));

    await userEvent.click(within(row).getByRole('button', { name: /track an intro/i }));
    await waitFor(() =>
      expect(api.callsTo('POST /intros')[0]?.body).toEqual({ person: 812, connector: null, stage: 'draft', note: null, follow_up: null }),
    );

    await userEvent.click(within(row).getByRole('button', { name: /remove from shortlist/i }));
    await waitFor(() => expect(api.callsTo('DELETE /shortlist/812')).toHaveLength(1));
  });

  it('empty shortlist', async () => {
    renderApp('#/shortlist', { 'GET /shortlist': { v: 1, shortlist: [] } });
    expect(await screen.findByText(/your next great conversation starts here/i)).toBeInTheDocument();
  });
});

describe('Introductions board', () => {
  it('shows lanes for every stage and the intro card in its lane', async () => {
    renderApp('#/intros');
    const asked = await screen.findByRole('region', { name: /^asked/i });
    for (const lane of ['Draft', 'Asked', 'Intro made', 'Met', 'Closed']) {
      expect(screen.getByRole('region', { name: new RegExp(`^${lane}`) })).toBeInTheDocument();
    }
    const card = within(asked).getByRole('link', { name: 'Kai Wu' }).closest('article')!;
    expect(within(card).getByText(/via Tom Okafor/)).toBeInTheDocument();
    expect(within(card).getByText(/follow up 2026-10-28/i)).toBeInTheDocument();
    expect(within(card).getByText('warm: same dinner series')).toBeInTheDocument();
    expect(screen.getByText(/never sends a message/i)).toBeInTheDocument();
  });

  it('moves an intro to another stage and removes it', async () => {
    const { api } = renderApp('#/intros', {
      'PATCH /intros/1': { v: 1, intro: { ...intro, stage: 'intro_made' } },
      'DELETE /intros/1': { v: 1, removed: true },
    });
    const asked = await screen.findByRole('region', { name: /^asked/i });
    await userEvent.selectOptions(within(asked).getByRole('combobox', { name: /stage for kai wu/i }), 'intro_made');
    await waitFor(() => expect(api.callsTo('PATCH /intros/1')[0]?.body).toEqual({ stage: 'intro_made' }));
    await userEvent.click(within(screen.getByRole('region', { name: /^asked/i })).getByRole('button', { name: /remove/i }));
    await waitFor(() => expect(api.callsTo('DELETE /intros/1')).toHaveLength(1));
  });

  it('creates an intro, suggesting connectors from how-connected', async () => {
    const { api } = renderApp('#/intros', {
      'POST /intros': { v: 1, intro: { ...intro, id: 5 } },
    });
    await screen.findByRole('region', { name: /^asked/i });
    await userEvent.click(screen.getByRole('button', { name: /new introduction/i }));
    const dialog = screen.getByRole('dialog');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /who do you want to meet/i }), 'Kai Wu');
    await userEvent.click(within(dialog).getByRole('button', { name: /suggest connectors/i }));
    await waitFor(() => expect(api.callsTo('GET /connections')[0]?.url.searchParams.get('target')).toBe('Kai Wu'));
    await userEvent.selectOptions(await within(dialog).findByRole('combobox', { name: /connector/i }), '812');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /^note/i }), 'same dinner series');
    await userEvent.type(within(dialog).getByLabelText(/follow-up/i), '2026-11-02');
    await userEvent.click(within(dialog).getByRole('button', { name: /save to introductions/i }));
    await waitFor(() =>
      expect(api.callsTo('POST /intros')[0]?.body).toEqual({
        person: 'Kai Wu',
        connector: 812,
        stage: 'draft',
        note: 'same dinner series',
        follow_up: '2026-11-02',
      }),
    );
  });
});
