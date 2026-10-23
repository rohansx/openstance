import { useMemo, useState } from 'react';
import type { Outcome, QuoteLine } from '../api/types';
import { configValue, useApp } from '../app/context';
import { href } from '../app/router';
import { PageHead } from '../components/bits';
import { PlaybookResult } from '../components/PlaybookResult';
import { Icon, type IconName } from '../components/Icon';
import { usd } from '../lib/format';

type FieldKind = 'text' | 'bool' | 'number' | 'lines';
interface FieldSpec {
  key: string;
  label: string;
  kind: FieldKind;
  required?: boolean;
  default?: string | boolean;
  placeholder?: string;
  /** number inputs: lower bound and step (default 1 and 1). */
  min?: number;
  step?: number;
}

interface PlaybookSpec {
  name: string;
  icon: IconName;
  title: string;
  summary: string;
  egress: 'none' | 'optional' | 'yes';
  egressNote: string;
  fields: FieldSpec[];
}

// Inputs from docs/tech-spec/playbooks.md.
const PLAYBOOKS: PlaybookSpec[] = [
  {
    name: 'path',
    icon: 'route',
    title: 'Path to a stranger',
    summary: 'The warmest route to someone, in your graph or not: ranked mutuals with evidence.',
    egress: 'yes',
    egressNote: 'Local first · a profile lookup for strangers · quote first',
    fields: [
      { key: 'target', label: 'Target', kind: 'text', required: true, placeholder: 'name, email, LinkedIn URL or id' },
      { key: 'company', label: 'Company', kind: 'text', placeholder: 'disambiguates the target' },
      { key: 'linkedin', label: 'LinkedIn', kind: 'text', placeholder: 'optional' },
      { key: 'find_email', label: 'Also find and verify their email', kind: 'bool', default: false },
      { key: 'limit', label: 'Paths', kind: 'number', default: '3' },
    ],
  },
  {
    name: 'brief',
    icon: 'mail',
    title: 'Intro brief',
    summary: 'A short, source-backed brief before you reach out: who they are, your history, who connects you.',
    egress: 'optional',
    egressNote: 'Local · optional web search · quote first',
    fields: [
      { key: 'person', label: 'Person', kind: 'text', required: true, placeholder: 'id, email or name' },
      { key: 'web', label: 'Include one web search for recent public posts', kind: 'bool', default: false },
    ],
  },
  {
    name: 'reconnect',
    icon: 'clock',
    title: 'Reconnect digest',
    summary: 'People you used to talk to often and have not lately, with the subject of your last exchange.',
    egress: 'none',
    egressNote: 'Local · $0 · no messages sent',
    fields: [
      { key: 'limit', label: 'People', kind: 'number', default: '10' },
      { key: 'min_frequency', label: 'Minimum past frequency F (0 to 1)', kind: 'number', default: '0.5', min: 0, step: 0.05 },
      { key: 'max_recency', label: 'Maximum recency R now (0 to 1)', kind: 'number', default: '0.2', min: 0, step: 0.05 },
    ],
  },
  {
    name: 'accounts',
    icon: 'people',
    title: 'Account mapping',
    summary: 'For each target company: current people, alumni, and one-hop paths, as a coverage table.',
    egress: 'none',
    egressNote: 'Local · $0',
    fields: [
      { key: 'companies', label: 'Companies (one per line)', kind: 'lines', required: true, placeholder: 'stripe.com\nRamp' },
      { key: 'one_hop', label: 'Include one-hop paths through your ties', kind: 'bool', default: true },
      { key: 'limit_per_company', label: 'People per company', kind: 'number', default: '10' },
    ],
  },
  {
    name: 'refresh',
    icon: 'spark',
    title: 'Refresh thin records',
    summary: 'Fill empty titles and companies for people you talk to, in score order, until the budget is used.',
    egress: 'yes',
    egressNote: 'Enrichment · respects no-enrich · quote first',
    fields: [
      { key: 'min_interactions', label: 'Minimum interactions', kind: 'number', default: '5' },
      { key: 'limit', label: 'People', kind: 'number', default: '50' },
    ],
  },
  {
    name: 'verify',
    icon: 'check',
    title: 'Verify before send',
    summary: 'Pick the best email for a person, verify it, and attach the most recent mutual who replied.',
    egress: 'yes',
    egressNote: '1 verification · quote first',
    fields: [{ key: 'person', label: 'Person', kind: 'text', required: true, placeholder: 'id, email or name' }],
  },
  {
    name: 'watch',
    icon: 'graph',
    title: 'Job-change watch',
    summary: 'Diff your latest LinkedIn exports for moves, and flag moves into your target companies.',
    egress: 'optional',
    egressNote: 'Local diff · optional profile checks · quote first',
    fields: [
      { key: 'targets', label: 'Target companies (one per line)', kind: 'lines', placeholder: 'Anthropic\nOpenAI' },
      { key: 'check_profiles', label: 'Also check top profiles through the enricher', kind: 'bool', default: false },
      { key: 'top_n', label: 'Profiles to check', kind: 'number', default: '100' },
    ],
  },
];

type Values = Record<string, string | boolean>;

function initialValues(spec: PlaybookSpec, params: Record<string, string>): Values {
  const v: Values = {};
  for (const f of spec.fields) {
    const p = params[f.key];
    if (f.kind === 'bool') v[f.key] = p !== undefined ? p === 'true' : Boolean(f.default);
    else v[f.key] = p ?? (typeof f.default === 'string' ? f.default : '');
  }
  return v;
}

function buildInput(spec: PlaybookSpec, values: Values): Record<string, unknown> {
  const input: Record<string, unknown> = {};
  for (const f of spec.fields) {
    const v = values[f.key];
    if (f.kind === 'bool') input[f.key] = v === true;
    else if (f.kind === 'number') {
      const n = Number(v);
      if (String(v).trim() !== '' && Number.isFinite(n)) input[f.key] = n;
    } else if (f.kind === 'lines') {
      const lines = String(v ?? '')
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean);
      if (lines.length) input[f.key] = lines;
    } else {
      const s = String(v ?? '').trim();
      if (s) input[f.key] = s;
    }
  }
  return input;
}

function lineSubject(l: QuoteLine): string {
  const r = l.job.request;
  for (const k of ['name', 'email', 'domain', 'query', 'linkedin']) {
    const v = r[k];
    if (typeof v === 'string' && v) return v;
  }
  return l.job.person_id !== null ? `person #${l.job.person_id}` : '–';
}

export function Playbooks({ selected, params }: { selected?: string; params: Record<string, string> }) {
  const { config } = useApp();
  const backend = String(configValue(config, 'enrich.backend') ?? 'none');
  const spec = PLAYBOOKS.find((p) => p.name === selected);

  return (
    <>
      <PageHead
        eyebrow="DETERMINISTIC STEPS. VISIBLE OUTPUTS."
        title="A next step, already thought through."
        sub="Every playbook runs its local steps for free and returns a quote for anything that would leave this machine. Nothing is spent until you press Spend."
        aside={
          <span className="status-badge">
            <Icon name="lock" /> Enricher: {backend === 'none' ? 'off' : backend}
          </span>
        }
      />
      {spec && <Runner key={spec.name} spec={spec} params={params} />}
      <ul className="playbook-grid" aria-label="Playbooks" style={{ listStyle: 'none', padding: 0, marginTop: spec ? 28 : 0 }}>
        {PLAYBOOKS.map((p) => (
          <li className="panel playbook-card" key={p.name}>
            <div className="panel-icon">
              <Icon name={p.icon} />
            </div>
            <span className={`tag ${p.egress === 'none' ? '' : 'warn'}`}>{p.egress === 'none' ? 'NO EGRESS' : 'MAY SPEND'}</span>
            <h2>
              {p.name} <small>{p.title}</small>
            </h2>
            <p>{p.summary}</p>
            <small>{p.egressNote}</small>
            <a className="btn primary" href={href({ name: 'playbooks', playbook: p.name })} aria-current={p.name === selected ? 'true' : undefined}>
              Open runner <Icon name="arrow" />
            </a>
          </li>
        ))}
      </ul>
    </>
  );
}

function Runner({ spec, params }: { spec: PlaybookSpec; params: Record<string, string> }) {
  const { api, toast, refreshStats } = useApp();
  const [values, setValues] = useState<Values>(() => initialValues(spec, params));
  const [cap, setCap] = useState('');
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  // the exact input the shown quote was made for
  const [quoted, setQuoted] = useState<{ key: string; input: Record<string, unknown>; cap: number | null } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const input = useMemo(() => buildInput(spec, values), [spec, values]);
  const capUsd = cap.trim() === '' ? null : Number(cap);
  const key = JSON.stringify([input, capUsd]);
  const stale = quoted !== null && quoted.key !== key;
  const missing = spec.fields.some((f) => f.required && input[f.key] === undefined);

  async function quote() {
    setBusy(true);
    setError(null);
    try {
      // The first call never allows egress: it returns the plan and the quote.
      const o = await api.playbook(spec.name, { input, allow_egress: false, max_usd: capUsd, plan_only: false });
      setOutcome(o);
      setQuoted({ key, input, cap: capUsd });
    } catch (e) {
      setOutcome(null);
      setQuoted(null);
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const q = outcome?.plan.quote;
  const canSpend =
    !!outcome &&
    !!q &&
    !!quoted &&
    !stale &&
    outcome.status === 'quote' &&
    q.lines.length > 0 &&
    // a $0 (free) quote still sends requests: it needs the same consent
    q.within_budget;

  // The only code path that allows egress (see the source-guard test).
  async function spend() {
    if (!canSpend || !q || !quoted) return;
    const quoteUsd = q.total_micro_usd / 1_000_000;
    const maxUsd = quoted.cap === null ? quoteUsd : Math.min(quoted.cap, quoteUsd);
    setBusy(true);
    setError(null);
    try {
      const o = await api.playbook(spec.name, { input: quoted.input, allow_egress: true, max_usd: maxUsd, plan_only: false });
      setOutcome(o);
      setQuoted(null);
      toast(`Sent ${o.receipt.requests_sent} request${o.receipt.requests_sent === 1 ? '' : 's'} · spent ${usd(o.receipt.spent_micro_usd)}.`);
      refreshStats();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const local = spec.egress === 'none';
  return (
    <section className="panel runner" aria-label={`Run ${spec.name}`}>
      <h2>
        {spec.name} <small>{spec.title}</small>
      </h2>
      <p>{spec.summary}</p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!missing) void quote();
        }}
      >
        {spec.fields.map((f) =>
          f.kind === 'bool' ? (
            <label className="check" key={f.key}>
              <input
                type="checkbox"
                checked={values[f.key] === true}
                onChange={(e) => setValues((v) => ({ ...v, [f.key]: e.target.checked }))}
              />
              {f.label}
            </label>
          ) : (
            <label className="field" key={f.key}>
              {f.label}
              {f.required ? ' *' : ''}
              {f.kind === 'lines' ? (
                <textarea
                  value={String(values[f.key] ?? '')}
                  placeholder={f.placeholder}
                  onChange={(e) => setValues((v) => ({ ...v, [f.key]: e.target.value }))}
                />
              ) : (
                <input
                  type={f.kind === 'number' ? 'number' : 'text'}
                  min={f.kind === 'number' ? (f.min ?? 1) : undefined}
                  step={f.kind === 'number' ? (f.step ?? 1) : undefined}
                  value={String(values[f.key] ?? '')}
                  placeholder={f.placeholder}
                  onChange={(e) => setValues((v) => ({ ...v, [f.key]: e.target.value }))}
                />
              )}
            </label>
          ),
        )}
        {!local && (
          <label className="field">
            Budget cap (USD, optional)
            <input
              type="number"
              min={0}
              step="0.01"
              value={cap}
              placeholder="uses enrich.max_usd_per_run"
              onChange={(e) => setCap(e.target.value)}
            />
          </label>
        )}
        <div className="form-actions">
          <button type="submit" className="btn primary" disabled={busy || missing}>
            {local ? 'Run playbook' : 'Get plan and quote'} <Icon name="arrow" />
          </button>
          {!local && <small className="muted">Free: runs local steps and prices the rest. Sends nothing.</small>}
        </div>
      </form>

      {error && (
        <div className="alert" role="alert">
          {error}
        </div>
      )}
      {outcome && <OutcomeView outcome={outcome} />}

      {outcome && q && outcome.status === 'quote' && (
        <section className="quote-box" aria-label="Quote">
          <h3>Quote · {q.backend}</h3>
          <div className="total">{usd(q.total_micro_usd)}</div>
          <p>
            {q.lines.length} request{q.lines.length === 1 ? '' : 's'} · budget {usd(q.budget_micro_usd)} · prices{' '}
            {[...new Set(q.lines.map((l) => l.basis))].join(', ') || '–'}
          </p>
          <ul className="notes-out">
            {q.lines.map((l, i) => (
              <li key={i}>
                {l.job.request.kind} · {lineSubject(l)} · {usd(l.micro_usd)}
                {l.provider_hint ? ` · ${l.provider_hint}` : ''}
              </li>
            ))}
            {q.skipped.map((s, i) => (
              <li key={`s${i}`}>
                skipped {s.op}
                {s.person_id !== null ? ` for #${s.person_id}` : ''}: {s.reason.replace(/_/g, ' ')}
              </li>
            ))}
          </ul>
          {!q.within_budget && (
            <p>
              <strong>This is over your budget</strong> ({usd(q.budget_micro_usd)}). Raise <code>enrich.max_usd_per_run</code>{' '}
              or narrow the input; nothing will be sent.
            </p>
          )}
          {stale && <p>Input changed: get a new quote before spending.</p>}
          <div className="gate-actions">
            {canSpend && (
              <button type="button" className="btn spend" disabled={busy} onClick={() => void spend()}>
                Spend {usd(q.total_micro_usd)}
              </button>
            )}
            <small className="muted">
              Sends only the fields listed above, capped at the quote. Every request is recorded in the{' '}
              <a href="#/egress">egress log</a>.
            </small>
          </div>
        </section>
      )}
    </section>
  );
}

function OutcomeView({ outcome }: { outcome: Outcome }) {
  const r = outcome.receipt;
  return (
    <div className="outcome">
      <h3 className="subtle-label">
        Plan · <span className="status-tag">{outcome.status.replace(/_/g, ' ')}</span>
      </h3>
      <div className="table-wrap">
        <table className="plan-steps">
          <thead>
            <tr>
              <th>#</th>
              <th>Step</th>
              <th>Egress</th>
              <th>Decision</th>
            </tr>
          </thead>
          <tbody>
            {outcome.plan.steps.map((s) => (
              <tr key={s.n}>
                <td>{s.n}</td>
                <td>{s.description}</td>
                <td>{s.egress ?? 'local'}</td>
                <td>{s.will_run}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {outcome.notes.length > 0 && (
        <ul className="notes-out">
          {outcome.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
      <PlaybookResult playbook={outcome.playbook} result={outcome.result} />
      <p className="receipt">
        <Icon name="shield" /> Requests sent: {r.requests_sent} · Spent: {usd(r.spent_micro_usd)} · Quoted:{' '}
        {usd(r.quoted_micro_usd)} · Budget: {usd(r.budget_micro_usd)} · run {r.run_id}
      </p>
    </div>
  );
}
