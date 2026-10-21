import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Outcome } from '../api/types';
import { fx } from '../test/fixtures';
import type { RecordedCall } from '../test/mockApi';
import { renderApp } from '../test/render';

// U4: playbook results are rendered human-first, with the raw JSON behind a
// toggle. U8: runner forms send the inputs of docs/tech-spec/playbooks.md.

function outcome(playbook: string, result: unknown, status: Outcome['status'] = 'done'): Outcome {
  const base = fx.quoteOutcome();
  return {
    ...base,
    playbook,
    status,
    plan: { ...base.plan, steps: [{ n: 1, description: 'local step', egress: null, will_run: 'run' }], quote: { ...base.plan.quote, lines: [], total_micro_usd: 0 } },
    result,
    notes: [],
  };
}

const tie = (id: number, name: string) => ({ kind: 'tie', with: { id, display_name: name }, tie_kind: 'email', n_shared: 69, weight: 0.998, last_at: '2026-10-14T14:25:12Z' });
const mail = {
  kind: 'interaction',
  interaction_id: 7020,
  type: 'email',
  direction: 'none',
  at: '2026-10-14T14:25:12Z',
  subject: 'Re: Corvid Analytics: Renewal discussion',
  role: 'from',
  source: { id: 3, kind: 'mbox', label: 'mbox 2026-10-15' },
  line: '2026-10-14  email  "Re: Corvid Analytics: Renewal discussion"  (mbox 2026-10-15)',
};

const BRIEF_MD = `**Delia Quintero** - Engineering Manager, Corvid Analytics (score 0.57)
- Last contact: 2026-10-14 email "Re: Corvid Analytics: Renewal discussion".
- Best mutual: Jordan Avery (score 0.56, 69 shared email, strength 0.56).
- Note: <script>alert(1)</script>`;

async function run(name: string, result: unknown, fill: (runner: HTMLElement) => Promise<void> = async () => {}) {
  const r = renderApp(`#/playbooks/${name}`, { [`POST /playbooks/${name}`]: outcome(name, result) });
  const runner = await screen.findByRole('region', { name: new RegExp(`run ${name}`, 'i') });
  await fill(runner);
  await userEvent.click(within(runner).getByRole('button', { name: /^(run playbook|get plan and quote)/i }));
  await within(runner).findByText(/requests sent: 0/i);
  return { ...r, runner, result: within(runner).getByRole('region', { name: /result/i }) };
}

const type = (label: RegExp, text: string) => async (runner: HTMLElement) => {
  await userEvent.type(within(runner).getByRole('textbox', { name: label }), text);
};

describe('playbook results, human-first (U4)', () => {
  beforeEach(() => {
    vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: vi.fn(() => 'blob:x'), revokeObjectURL: vi.fn() }));
  });

  it('brief: rendered markdown (no raw HTML), with Copy and Download .md', async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const { runner, result } = await run(
      'brief',
      { markdown: BRIEF_MD, person: { id: 182, display_name: 'Delia Quintero' }, interactions: [mail], mutuals: [], web: [] },
      type(/^person/i, '182'),
    );
    expect(within(result).getByText('Delia Quintero').tagName).toBe('STRONG');
    expect(within(result).getAllByRole('listitem').map((li) => li.textContent)).toContain('Best mutual: Jordan Avery (score 0.56, 69 shared email, strength 0.56).');
    expect(runner.querySelector('script')).toBeNull();
    expect(within(result).getAllByText(/<script>alert\(1\)<\/script>/).filter((el) => !el.closest('pre'))).toHaveLength(1);
    // the raw JSON is collapsed, not the main view
    const raw = within(runner).getByText('Raw JSON').closest('details')!;
    expect(raw).not.toHaveAttribute('open');

    await userEvent.click(within(result).getByRole('button', { name: /copy/i }));
    expect(writeText).toHaveBeenCalledWith(BRIEF_MD);

    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    await userEvent.click(within(result).getByRole('button', { name: /download \.md/i }));
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
    const blob = vi.mocked(URL.createObjectURL).mock.calls[0]![0] as Blob;
    expect(blob.type).toBe('text/markdown');
    expect(await blob.text()).toBe(BRIEF_MD);
    const a = click.mock.contexts[0] as HTMLAnchorElement;
    expect(a.download).toBe('brief-delia-quintero.md');
    click.mockRestore();
  });

  it('path: ranked path cards with strength, reason and evidence', async () => {
    const { result } = await run(
      'path',
      {
        target: { person_id: 182, display_name: 'Delia Quintero', company: 'Corvid Analytics', in_graph: true, profile_source: null },
        direct: { score: 0.568, last_contact: '2026-10-14T14:25:12Z' },
        paths: [
          { via: { id: 775, display_name: 'Jordan Avery', title: null, company: null, score: 0.562 }, strength: 0.561, reason: 'tie', my_score: 0.562, link_weight: 0.998, evidence: [tie(182, 'Delia Quintero'), mail] },
          { via: { id: 896, display_name: 'Lars Mbeki', title: 'CTO', company: 'Corvid Analytics', score: 0.4 }, strength: 0.31, reason: 'same_company', my_score: 0.4, link_weight: 0.78, evidence: [] },
        ],
        email: { address: 'delia@corvid.example', status: 'deliverable', source: 'enrich:treg' },
      },
      type(/^target/i, '182'),
    );
    expect(result.querySelector('h4')).toHaveTextContent(/Delia Quintero · Corvid Analytics in your graph/i);
    expect(within(result).getByText(/you know them directly · score 0\.57/i)).toBeInTheDocument();
    const cards = within(within(result).getByRole('list', { name: /paths/i })).getAllByRole('listitem', { name: /^path/i });
    expect(cards).toHaveLength(2);
    expect(within(cards[0]!).getByRole('link', { name: 'Jordan Avery' })).toHaveAttribute('href', '#/people/775');
    expect(cards[0]).toHaveTextContent('1');
    expect(cards[0]).toHaveTextContent(/strength 0\.561/);
    expect(cards[0]).toHaveTextContent(/shared threads/i);
    expect(cards[0]).toHaveTextContent(/2026-10-14 · email · "Re: Corvid Analytics: Renewal discussion"|Re: Corvid Analytics: Renewal discussion/);
    expect(cards[0]).toHaveTextContent(/tie with Delia Quintero · 69 shared email/);
    expect(cards[1]).toHaveTextContent(/same company/i);
    expect(cards[1]).toHaveTextContent(/CTO · Corvid Analytics/);
    expect(within(result).getByText(/^Email:/)).toHaveTextContent('Email: delia@corvid.example · deliverable (enrich:treg)');
  });

  it('path with no paths says so', async () => {
    const { result } = await run(
      'path',
      { target: { person_id: null, display_name: 'Priya Raman', company: null, in_graph: false, profile_source: null }, direct: null, paths: [], email: null },
      type(/^target/i, 'Priya Raman'),
    );
    expect(within(result).getByText(/not in your graph/i)).toBeInTheDocument();
    expect(within(result).getByText(/no paths found/i)).toBeInTheDocument();
  });

  it('reconnect: a table of people with was/now scores and the last exchange', async () => {
    const { result } = await run('reconnect', {
      people: [
        { id: 5, display_name: 'Maya Chen', past_score: 0.74, score: 0.41, last_contact: '2025-10-02', last_subject: 'Re: seed round notes' },
        { id: 6, display_name: 'Luis Ortega', past_score: 0.7, score: 0.38, last_contact: '2025-08-19', last_subject: 'lunch?' },
      ],
    });
    const table = within(result).getByRole('table', { name: /reconnect/i });
    const rows = within(table).getAllByRole('row');
    expect(rows).toHaveLength(3);
    expect(within(rows[1]!).getByRole('link', { name: 'Maya Chen' })).toHaveAttribute('href', '#/people/5');
    expect(rows[1]).toHaveTextContent(/0\.74.*0\.41.*2025-10-02.*Re: seed round notes/);
  });

  it('accounts: a coverage table with the best contact and per-company people', async () => {
    const { result } = await run(
      'accounts',
      {
        companies: [
          {
            company: 'Stripe',
            domain: 'stripe.com',
            current: [{ id: 812, display_name: 'Tom Okafor', title: 'EM', score: 0.78 }],
            alumni: [{ id: 77, display_name: 'Ana Lee', left_seen: '2024-08', score: 0.61 }],
            one_hop: [{ id: 1203, display_name: 'Kai Wu', via: 812, strength: 0.33 }],
            coverage: 'warm',
            best: 812,
          },
          { company: 'Ramp', domain: null, current: [], alumni: [], one_hop: [], coverage: 'none', best: null },
        ],
      },
      async (runner) => {
        await userEvent.type(within(runner).getByRole('textbox', { name: /companies/i }), 'stripe.com{Enter}Ramp');
      },
    );
    const table = within(result).getByRole('table', { name: /coverage/i });
    const rows = within(table).getAllByRole('row');
    expect(rows[1]).toHaveTextContent(/Stripe.*1.*1.*1.*warm.*Tom Okafor \(0\.78\)/);
    expect(rows[2]).toHaveTextContent(/Ramp.*0.*0.*0.*none/);
    expect(within(rows[1]!).getByRole('link', { name: /tom okafor/i })).toHaveAttribute('href', '#/people/812');
    expect(within(result).getByText(/^Alumni:/)).toHaveTextContent(/Ana Lee \(alumni, left 2024-08\)/);
    expect(within(result).getByText(/^One hop:/)).toHaveTextContent(/Kai Wu \(via #812, strength 0\.33\)/);
  });

  it('refresh: updated people with their new fields, and the ones not found', async () => {
    const { result } = await run('refresh', {
      updated: [{ id: 31, display_name: 'Sam Ito', fields: { title: 'Head of Data', company: 'Ramp' } }],
      not_found: [44],
      skipped: [{ op: 'profile', person_id: 45, reason: 'no_enrich' }],
    });
    const table = within(result).getByRole('table', { name: /updated/i });
    expect(within(table).getByRole('link', { name: 'Sam Ito' })).toHaveAttribute('href', '#/people/31');
    expect(table).toHaveTextContent(/title: Head of Data/);
    expect(table).toHaveTextContent(/company: Ramp/);
    expect(within(result).getByRole('link', { name: '#44' })).toHaveAttribute('href', '#/people/44');
    expect(within(result).getByText(/#45.*no enrich/)).toBeInTheDocument();
  });

  it('verify: the address, its status and the freshest mutual who replied', async () => {
    const { result } = await run(
      'verify',
      { email: 'priya@stripe.example', status: 'deliverable', verified_at: '2026-10-20T10:00:00Z', mutual: { id: 812, display_name: 'Tom Okafor', last_reply: '2026-08-30' } },
      type(/^person/i, 'Priya Raman'),
    );
    expect(within(result).getByText('priya@stripe.example')).toBeInTheDocument();
    expect(within(result).getByText('deliverable')).toBeInTheDocument();
    expect(within(result).getByText(/verified 2026-10-20/)).toBeInTheDocument();
    expect(within(result).getByRole('link', { name: 'Tom Okafor' })).toHaveAttribute('href', '#/people/812');
    expect(within(result).getByText(/last reply 2026-08-30/)).toBeInTheDocument();
  });

  it('watch: moves, with moves into a target company highlighted', async () => {
    const { result } = await run('watch', {
      moves: [
        { id: 77, display_name: 'Ana Lee', from: 'Stripe', to: 'Anthropic', detected_by: 'linkedin_diff', target_hit: true },
        { id: 78, display_name: 'Bo Park', from: 'Ramp', to: 'Plaid', detected_by: 'linkedin_diff', target_hit: false },
      ],
      checked: 0,
    });
    const table = within(result).getByRole('table', { name: /moves/i });
    const rows = within(table).getAllByRole('row');
    expect(rows[1]).toHaveTextContent(/Ana Lee.*Stripe.*Anthropic.*linkedin diff.*target/i);
    expect(within(rows[2]!).queryByText(/^target$/i)).toBeNull();
    expect(within(result).getByText(/1 move into your target companies/i)).toBeInTheDocument();
    expect(within(result).getByText(/0 profiles checked/i)).toBeInTheDocument();
  });

  it('the Raw JSON toggle shows the exact result', async () => {
    const people = [{ id: 5, display_name: 'Maya Chen', past_score: 0.74, score: 0.41, last_contact: '2025-10-02', last_subject: 'x' }];
    const { runner } = await run('reconnect', { people });
    const summary = within(runner).getByText('Raw JSON');
    await userEvent.click(summary);
    expect(summary.closest('details')).toHaveAttribute('open');
    expect(runner.querySelector('pre.result-json')!.textContent).toBe(JSON.stringify({ people }, null, 2));
  });

  it('an unrecognized result shape falls back to the raw JSON, open', async () => {
    const { runner } = await run('reconnect', { something: 'else' });
    expect(within(runner).getByText('Raw JSON').closest('details')).toHaveAttribute('open');
  });
});

describe('runner inputs match playbooks.md (U8)', () => {
  const bodies = (calls: RecordedCall[]) => calls.map((c) => (c.body as { input: Record<string, unknown> }).input);

  it('reconnect sends limit, min_frequency and max_recency', async () => {
    const { api } = await run('reconnect', { people: [] });
    expect(bodies(api.callsTo('POST /playbooks/reconnect'))[0]).toEqual({ limit: 10, min_frequency: 0.5, max_recency: 0.2 });
  });

  it('accounts, refresh, verify and watch send the spec inputs', async () => {
    let r = await run('accounts', { companies: [] }, async (runner) => {
      await userEvent.type(within(runner).getByRole('textbox', { name: /companies/i }), 'stripe.com{Enter}Anthropic');
    });
    expect(bodies(r.api.callsTo('POST /playbooks/accounts'))[0]).toEqual({ companies: ['stripe.com', 'Anthropic'], one_hop: true, limit_per_company: 10 });
    r.unmount();

    r = await run('refresh', { updated: [], not_found: [], skipped: [] });
    expect(bodies(r.api.callsTo('POST /playbooks/refresh'))[0]).toEqual({ min_interactions: 5, limit: 50 });
    r.unmount();

    r = await run('verify', { email: null }, type(/^person/i, 'priya@stripe.example'));
    expect(bodies(r.api.callsTo('POST /playbooks/verify'))[0]).toEqual({ person: 'priya@stripe.example' });
    r.unmount();

    r = await run('watch', { moves: [], checked: 0 }, async (runner) => {
      await userEvent.type(within(runner).getByRole('textbox', { name: /target companies/i }), 'Anthropic{Enter}OpenAI');
    });
    expect(bodies(r.api.callsTo('POST /playbooks/watch'))[0]).toEqual({ targets: ['Anthropic', 'OpenAI'], check_profiles: false, top_n: 100 });
  });

  it('a server error for a playbook it does not support yet is a clear message, not a crash', async () => {
    renderApp('#/playbooks/watch', {
      'POST /playbooks/watch': { status: 404, body: { v: 1, error: { code: 'not_found', message: 'not found: playbook "watch"' } } },
    });
    const runner = await screen.findByRole('region', { name: /run watch/i });
    await userEvent.click(within(runner).getByRole('button', { name: /get plan and quote/i }));
    await waitFor(() => expect(within(runner).getByRole('alert')).toHaveTextContent('not found: playbook "watch"'));
  });
});
