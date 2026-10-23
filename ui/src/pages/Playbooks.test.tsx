import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { Outcome } from '../api/types';
import { fx } from '../test/fixtures';
import type { RecordedCall } from '../test/mockApi';
import { renderApp } from '../test/render';

type Body = { input: Record<string, unknown>; allow_egress: boolean; max_usd: number | null; plan_only: boolean };
const bodies = (calls: RecordedCall[]) => calls.map((c) => c.body as Body);

/** path: quote first, then (only with allow_egress) the done outcome. */
function pathRoute(call: RecordedCall) {
  return (call.body as Body).allow_egress ? fx.doneOutcome() : fx.quoteOutcome();
}

async function quotePath() {
  const r = renderApp('#/playbooks/path', { 'POST /playbooks/path': pathRoute });
  const runner = await screen.findByRole('region', { name: /run path/i });
  await userEvent.type(within(runner).getByRole('textbox', { name: /^target/i }), 'Priya Raman');
  await userEvent.type(within(runner).getByRole('textbox', { name: /^company/i }), 'Stripe');
  await userEvent.click(within(runner).getByRole('button', { name: /get plan and quote/i }));
  await within(runner).findByText(/requests sent: 0/i);
  return { ...r, runner };
}

describe('Playbook runner with quote gate (P4-UI-06)', () => {
  it('lists the playbooks with honest egress labels', async () => {
    renderApp('#/playbooks');
    const grid = await screen.findByRole('list', { name: /playbooks/i });
    for (const name of ['path', 'brief', 'reconnect', 'accounts', 'refresh', 'verify', 'watch']) {
      expect(within(grid).getByRole('heading', { name: new RegExp(`^${name}`, 'i') })).toBeInTheDocument();
    }
    const reconnect = within(grid).getByRole('heading', { name: /^reconnect/i }).closest('li')!;
    expect(within(reconnect).getByText(/local · \$0/i)).toBeInTheDocument();
    const path = within(grid).getByRole('heading', { name: /^path/i }).closest('li')!;
    expect(within(path).getByText(/quote first/i)).toBeInTheDocument();
  });

  it('the first run sends allow_egress:false and shows the plan, quote and notes', async () => {
    const { api, runner } = await quotePath();
    const calls = bodies(api.callsTo('POST /playbooks/path'));
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      input: { target: 'Priya Raman', company: 'Stripe', find_email: false, limit: 3 },
      allow_egress: false,
      max_usd: null,
      plan_only: false,
    });
    expect(within(runner).getByText('lookup target in graph')).toBeInTheDocument();
    expect(within(runner).getByText('skip_no_egress')).toBeInTheDocument();
    expect(within(runner).getByText(/egress not allowed/)).toBeInTheDocument();
    const quote = within(runner).getByRole('region', { name: /quote/i });
    expect(within(quote).getByText('$0.01')).toBeInTheDocument();
    expect(within(quote).getByText(/treg/)).toBeInTheDocument();
    expect(within(quote).getByText(/profile · Priya Raman/)).toBeInTheDocument();
  });

  it('allow_egress:true is sent only after clicking "Spend $0.01", capped at the quote', async () => {
    const { api, runner } = await quotePath();
    expect(bodies(api.callsTo('POST /playbooks/path')).some((b) => b.allow_egress)).toBe(false);

    const spend = within(runner).getByRole('button', { name: 'Spend $0.01' });
    await userEvent.click(spend);

    await within(runner).findByText(/requests sent: 1/i);
    const calls = bodies(api.callsTo('POST /playbooks/path'));
    expect(calls).toHaveLength(2);
    expect(calls[1]).toEqual({
      input: { target: 'Priya Raman', company: 'Stripe', find_email: false, limit: 3 },
      allow_egress: true,
      max_usd: 0.01,
      plan_only: false,
    });
    expect(within(runner).getByText(/spent: \$0\.01/i)).toBeInTheDocument();
    expect(within(runner).queryByRole('button', { name: /^spend/i })).toBeNull();
  });

  it('changing the input after a quote withdraws the Spend button', async () => {
    const { runner } = await quotePath();
    expect(within(runner).getByRole('button', { name: 'Spend $0.01' })).toBeInTheDocument();
    await userEvent.type(within(runner).getByRole('textbox', { name: /^company/i }), ' Inc');
    expect(within(runner).queryByRole('button', { name: /^spend/i })).toBeNull();
    expect(within(runner).getByText(/input changed: get a new quote/i)).toBeInTheDocument();
  });

  it('no Spend button for an over-budget quote', async () => {
    const over = fx.quoteOutcome();
    over.plan.quote.within_budget = false;
    over.plan.quote.budget_micro_usd = 5000;
    renderApp('#/playbooks/path', { 'POST /playbooks/path': over });
    const runner = await screen.findByRole('region', { name: /run path/i });
    await userEvent.type(within(runner).getByRole('textbox', { name: /^target/i }), 'Priya Raman');
    await userEvent.click(within(runner).getByRole('button', { name: /get plan and quote/i }));
    expect(await within(runner).findByText(/over your budget/i)).toBeInTheDocument();
    expect(within(runner).queryByRole('button', { name: /^spend/i })).toBeNull();
  });

  it('a free quote that still sends requests needs the Spend button too (refresh with free lines)', async () => {
    // engine shape: treg quotes a profile with no LinkedIn or company at $0 (basis free)
    const free = (body: Body): Outcome => {
      const o = fx.quoteOutcome();
      const line = { ...o.plan.quote.lines[0]!, micro_usd: 0, basis: 'free' as const };
      return {
        ...o,
        playbook: 'refresh',
        status: body.allow_egress ? 'partial' : 'quote',
        plan: { ...o.plan, playbook: 'refresh', quote: { ...o.plan.quote, lines: [line, line, line], total_micro_usd: 0, within_budget: true } },
        result: { updated: [], unchanged: [], not_found: [], failed: [], skipped: [] },
        receipt: { ...o.receipt, requests_sent: 0, spent_micro_usd: 0 },
        notes: [],
      };
    };
    const { api } = renderApp('#/playbooks/refresh', { 'POST /playbooks/refresh': (c: RecordedCall) => free(c.body as Body) });
    const runner = await screen.findByRole('region', { name: /run refresh/i });
    await userEvent.click(within(runner).getByRole('button', { name: /get plan and quote/i }));
    const spend = await within(runner).findByRole('button', { name: 'Spend $0.00' });
    expect(bodies(api.callsTo('POST /playbooks/refresh')).some((b) => b.allow_egress)).toBe(false);
    await userEvent.click(spend);
    await waitFor(() => expect(bodies(api.callsTo('POST /playbooks/refresh'))).toHaveLength(2));
    expect(bodies(api.callsTo('POST /playbooks/refresh'))[1]).toMatchObject({ allow_egress: true, max_usd: 0 });
  });

  it('local playbooks run with allow_egress:false and never offer to spend', async () => {
    const reconnect: Outcome = {
      ...fx.quoteOutcome(),
      playbook: 'reconnect',
      status: 'done',
      plan: { ...fx.quoteOutcome().plan, steps: [{ n: 1, description: 'rank lost-touch people', egress: null, will_run: 'run' }], quote: { ...fx.quoteOutcome().plan.quote, lines: [], total_micro_usd: 0 }, egress_max: [] },
      result: { people: [{ id: 812, display_name: 'Tom Okafor' }] },
      notes: [],
    };
    const { api } = renderApp('#/playbooks/reconnect', { 'POST /playbooks/reconnect': reconnect });
    const runner = await screen.findByRole('region', { name: /run reconnect/i });
    await userEvent.click(within(runner).getByRole('button', { name: /^run/i }));
    expect(await within(runner).findByRole('link', { name: 'Tom Okafor' })).toHaveAttribute('href', '#/people/812');
    expect(bodies(api.callsTo('POST /playbooks/reconnect'))[0]).toEqual({
      input: { limit: 10, min_frequency: 0.5, max_recency: 0.2 },
      allow_egress: false,
      max_usd: null,
      plan_only: false,
    });
    expect(within(runner).queryByRole('button', { name: /^spend/i })).toBeNull();
  });

  it('prefills from the link on a person page', async () => {
    renderApp('#/playbooks/brief?person=812');
    const runner = await screen.findByRole('region', { name: /run brief/i });
    expect(within(runner).getByRole('textbox', { name: /^person/i })).toHaveValue('812');
  });

  it('shows API errors such as egress_disabled', async () => {
    renderApp('#/playbooks/verify', {
      'POST /playbooks/verify': { status: 409, body: { v: 1, error: { code: 'egress_disabled', message: 'enrich.backend is "none"' } } },
    });
    const runner = await screen.findByRole('region', { name: /run verify/i });
    await userEvent.type(within(runner).getByRole('textbox', { name: /^person/i }), 'priya@stripe.example');
    await userEvent.click(within(runner).getByRole('button', { name: /get plan and quote/i }));
    expect(await within(runner).findByRole('alert')).toHaveTextContent('enrich.backend is "none"');
  });

  it('a budget cap is sent with the quote request and lowers the spend cap', async () => {
    const { api } = renderApp('#/playbooks/path', { 'POST /playbooks/path': pathRoute });
    const runner = await screen.findByRole('region', { name: /run path/i });
    await userEvent.type(within(runner).getByRole('textbox', { name: /^target/i }), 'Priya Raman');
    await userEvent.type(within(runner).getByRole('spinbutton', { name: /budget cap/i }), '0.5');
    await userEvent.click(within(runner).getByRole('button', { name: /get plan and quote/i }));
    await within(runner).findByRole('button', { name: 'Spend $0.01' });
    expect(bodies(api.callsTo('POST /playbooks/path'))[0]!.max_usd).toBe(0.5);
    await userEvent.click(within(runner).getByRole('button', { name: 'Spend $0.01' }));
    await waitFor(() => expect(bodies(api.callsTo('POST /playbooks/path'))[1]!.max_usd).toBe(0.01));
  });
});

describe('quote gate: source guard', () => {
  it('exactly one place in the app source can send allow_egress: true (the Spend handler)', () => {
    const sources = import.meta.glob(['../**/*.{ts,tsx}', '!../**/*.test.{ts,tsx}', '!../test/**'], {
      query: '?raw',
      import: 'default',
      eager: true,
    }) as Record<string, string>;
    const hits = Object.entries(sources).flatMap(([file, text]) =>
      (text.match(/allow_egress:\s*true/g) ?? []).map(() => file),
    );
    expect(hits).toEqual(['./Playbooks.tsx']);
    // and the client defaults to false unless told otherwise
    expect(sources['../api/client.ts']).toMatch(/allow_egress:\s*req\.allow_egress === true/);
  });
});
