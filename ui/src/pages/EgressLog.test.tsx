import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { renderApp } from '../test/render';

describe('Egress log (P4-UI-05)', () => {
  it('is a read-only table of every request with totals', async () => {
    renderApp('#/egress');
    const table = await screen.findByRole('table', { name: /egress log/i });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(3);
    expect(within(rows[0]!).getByText('profile')).toBeInTheDocument();
    expect(within(rows[0]!).getByText('Priya Raman')).toBeInTheDocument();
    expect(within(rows[0]!).getByText('ok')).toBeInTheDocument();
    expect(within(rows[2]!).getAllByText('$0.005')).toHaveLength(2);
    const totals = screen.getByRole('region', { name: /totals/i });
    expect(within(totals).getByText('3')).toBeInTheDocument();
    expect(within(totals).getByText('$0.025')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /delete|edit/i })).toBeNull();
  });

  it('filters by time window and run', async () => {
    const { api } = renderApp('#/egress');
    await screen.findByRole('table', { name: /egress log/i });
    await userEvent.selectOptions(screen.getByRole('combobox', { name: /window/i }), '30d');
    await waitFor(() => expect(api.callsTo('GET /enrich-log').at(-1)!.url.searchParams.get('since')).toBe('30d'));
    await userEvent.click(screen.getAllByRole('button', { name: /only run 01JC4Z6Y3R1M8Q/i })[0]!);
    await waitFor(() => expect(api.callsTo('GET /enrich-log').at(-1)!.url.searchParams.get('run')).toBe('01JC4Z6Y3R1M8Q'));
  });

  it('says plainly when nothing has left the machine', async () => {
    renderApp('#/egress', { 'GET /enrich-log': { v: 1, rows: [], totals: { requests_sent: 0, spent_micro_usd: 0, quoted_only: 0, blocked: 0 } } });
    expect(await screen.findByText(/nothing has left this machine/i)).toBeInTheDocument();
  });
});
