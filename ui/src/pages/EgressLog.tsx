import { useState } from 'react';
import type { EnrichLogRow } from '../api/types';
import { useApp } from '../app/context';
import { Empty, ErrorNote, Loading, PageHead } from '../components/bits';
import { Icon } from '../components/Icon';
import { usd } from '../lib/format';
import { useLoad } from '../lib/useLoad';

function subject(r: EnrichLogRow): string {
  for (const k of ['name', 'email', 'domain', 'query']) {
    const v = r.request?.[k];
    if (typeof v === 'string' && v) return v;
  }
  return r.person_name ?? (r.person_id !== null ? `#${r.person_id}` : '–');
}

const fmtAt = (iso: string) => iso.replace('T', ' ').replace(/:\d\dZ$/, '');

export function EgressLog() {
  const { api } = useApp();
  const [since, setSince] = useState('');
  const [run, setRun] = useState('');
  const log = useLoad(() => api.enrichLog({ since: since || undefined, run: run || undefined }), [api, since, run]);
  const rows = log.data?.rows ?? [];
  const totals = log.data?.totals;

  return (
    <>
      <PageHead
        eyebrow="EVERYTHING THAT LEFT, AND WHAT IT COST"
        title="The egress log."
        sub="Every request priced or sent to an enrichment provider or hosted model, with exactly the fields it carried. Read-only: this is your audit trail."
        aside={
          <span className="status-badge">
            <Icon name="lock" /> Read-only
          </span>
        }
      />
      <section className="totals" aria-label="Totals">
        <div>
          <strong>{totals?.requests_sent ?? '–'}</strong>
          <span>requests sent</span>
        </div>
        <div>
          <strong>{totals ? usd(totals.spent_micro_usd) : '–'}</strong>
          <span>spent</span>
        </div>
        <div>
          <strong>{totals?.quoted_only ?? '–'}</strong>
          <span>quoted, never sent</span>
        </div>
        <div>
          <strong>{totals?.blocked ?? '–'}</strong>
          <span>blocked by the gate</span>
        </div>
      </section>
      <div className="log-filters">
        <select aria-label="Window" value={since} onChange={(e) => setSince(e.target.value)}>
          <option value="">All time</option>
          <option value="7d">Last 7 days</option>
          <option value="30d">Last 30 days</option>
          <option value="90d">Last 90 days</option>
        </select>
        {run && (
          <button type="button" className="btn" onClick={() => setRun('')}>
            Run {run} ×
          </button>
        )}
      </div>
      <ErrorNote error={log.error} />
      {log.loading && !log.data && <Loading what="Loading the egress log" />}
      {log.data && !rows.length && (
        <Empty title="Nothing has left this machine." icon="shield">
          No enrichment or hosted-model request has been priced or sent{since ? ' in this window' : ''}.
        </Empty>
      )}
      {rows.length > 0 && (
        <div className="table-wrap panel">
          <table aria-label="Egress log">
            <thead>
              <tr>
                <th>At (UTC)</th>
                <th>Run</th>
                <th>Playbook</th>
                <th>Op</th>
                <th>About</th>
                <th>Backend</th>
                <th>Status</th>
                <th>Quoted</th>
                <th>Cost</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.id}>
                  <td>{fmtAt(r.at)}</td>
                  <td>
                    <button
                      type="button"
                      className="btn text"
                      aria-label={`Only run ${r.run_id}`}
                      title="Show only this run"
                      onClick={() => setRun(r.run_id)}
                    >
                      <code>{r.run_id.slice(0, 10)}</code>
                    </button>
                  </td>
                  <td>{r.playbook ?? '–'}</td>
                  <td>{r.op}</td>
                  <td>{subject(r)}</td>
                  <td>
                    {r.backend}
                    {r.provider ? ` · ${r.provider}` : ''}
                  </td>
                  <td>
                    <span className={`tag ${r.status === 'ok' ? '' : r.status === 'error' || r.status === 'blocked' ? 'warn' : 'neutral'}`}>
                      {r.status}
                    </span>
                    {r.error ? <div className="muted">{r.error}</div> : null}
                  </td>
                  <td>{usd(r.quoted_micro_usd)}</td>
                  <td>{r.cost_micro_usd === null ? '–' : usd(r.cost_micro_usd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="note">
        Also available as <code>openstance enrich log --json</code>. To delete what was learned about a person, run{' '}
        <code>openstance enrich forget &lt;person&gt;</code>.
      </p>
    </>
  );
}
