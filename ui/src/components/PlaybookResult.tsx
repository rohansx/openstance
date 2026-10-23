import type { ReactNode } from 'react';
import type { Evidence, PathJson, PersonRef, Skipped } from '../api/types';
import { useApp } from '../app/context';
import { href } from '../app/router';
import { downloadBlob } from '../lib/download';
import { displayName, shortDate } from '../lib/format';
import { Markdown } from '../lib/markdown';
import { EvidenceList } from './bits';
import { Icon } from './Icon';

// Playbook `result` rendered human-first (U4). Shapes from
// docs/tech-spec/playbooks.md sections 2-8 and interfaces.md 1.7. Every
// renderer is defensive: an unexpected shape returns null and the caller
// shows the raw JSON instead.

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v);
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const score = (v: unknown): string => (num(v) === null ? '–' : num(v)!.toFixed(2));

interface Ref {
  id: number | null;
  display_name: string | null;
}

function ref(v: unknown): Ref | null {
  if (!isObj(v)) return null;
  const id = num(v.id) ?? num(v.person_id);
  return { id, display_name: str(v.display_name) };
}

function PersonLink({ person }: { person: Ref | null }) {
  if (!person) return <>–</>;
  const name = person.display_name || (person.id !== null ? `#${person.id}` : 'Unknown');
  if (person.id === null) return <>{name}</>;
  return <a href={href({ name: 'person', id: String(person.id) })}>{name}</a>;
}

const REASON: Record<string, string> = {
  tie: 'shared threads',
  same_company: 'same company',
  past_company: 'former colleague',
  school: 'same school',
};

// ---------------------------------------------------------------------------

function Brief({ result }: { result: Obj }) {
  const { toast } = useApp();
  const md = str(result.markdown);
  if (md === null) return null;
  const who = ref(result.person);
  const slug =
    (who?.display_name ?? (who?.id !== null && who?.id !== undefined ? String(who.id) : 'person'))
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '') || 'person';
  return (
    <>
      <Markdown source={md} className="markdown brief-md" />
      <div className="result-actions-row">
        <button
          type="button"
          className="btn"
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(md);
              toast('Brief copied as markdown.');
            } catch {
              toast('Could not copy; select the text and copy it manually.');
            }
          }}
        >
          <Icon name="code" />
          Copy
        </button>
        <button type="button" className="btn" onClick={() => downloadBlob(new Blob([md], { type: 'text/markdown' }), `brief-${slug}.md`)}>
          <Icon name="upload" />
          Download .md
        </button>
      </div>
    </>
  );
}

function PathResult({ result }: { result: Obj }) {
  if (!Array.isArray(result.paths)) return null;
  const paths = result.paths.filter(isObj) as unknown as PathJson[];
  const target = isObj(result.target) ? result.target : {};
  const direct = isObj(result.direct) ? result.direct : null;
  const email = isObj(result.email) ? result.email : null;
  const name = str(target.display_name) ?? 'Target';
  const inGraph = target.in_graph === true;
  return (
    <>
      <div className="path-target">
        <h4>
          {inGraph && num(target.person_id) !== null ? <PersonLink person={{ id: num(target.person_id), display_name: name }} /> : name}
          {str(target.company) ? ` · ${String(target.company)}` : ''} <small>{inGraph ? 'in your graph' : 'not in your graph'}</small>
        </h4>
        {direct && (
          <p className="muted">
            You know them directly · score {score(direct.score)}
            {str(direct.last_contact) ? ` · last contact ${shortDate(String(direct.last_contact))}` : ''}
          </p>
        )}
        {str(target.profile_source) && <p className="muted">Profile from {String(target.profile_source)}</p>}
      </div>
      {paths.length === 0 ? (
        <p className="muted">No paths found through people in your graph.</p>
      ) : (
        <ol className="path-cards" aria-label="Paths">
          {paths.map((p, i) => {
            const via = p.via as PersonRef & { title?: string | null };
            const role = [via.title, via.company].filter(Boolean).join(' · ');
            return (
              <li className="path-card" key={`${via.id}-${i}`} aria-label={`Path ${i + 1} via ${displayName(via)}`}>
                <div className="path-card-head">
                  <span className="rank">{i + 1}</span>
                  <div>
                    <strong>
                      <PersonLink person={via} />
                    </strong>
                    {role && <small>{role}</small>}
                  </div>
                  <span className="strength">strength {num(p.strength)?.toFixed(3) ?? '–'}</span>
                </div>
                <p className="muted">
                  {REASON[p.reason] ?? String(p.reason ?? '').replace(/_/g, ' ')} · your score {score(p.my_score)} · link{' '}
                  {score(p.link_weight)}
                </p>
                <EvidenceList items={arr(p.evidence).filter(isObj) as unknown as Evidence[]} empty="No evidence lines for this path." />
              </li>
            );
          })}
        </ol>
      )}
      {email && str(email.address) && (
        <p>
          Email: <code>{String(email.address)}</code> · {str(email.status) ?? 'unverified'}
          {str(email.source) ? ` (${String(email.source)})` : ''}
        </p>
      )}
    </>
  );
}

function Reconnect({ result }: { result: Obj }) {
  if (!Array.isArray(result.people)) return null;
  const people = result.people.filter(isObj);
  if (!people.length) return <p className="muted">Nobody to reconnect with right now.</p>;
  return (
    <div className="table-wrap">
      <table aria-label="Reconnect digest">
        <thead>
          <tr>
            <th>#</th>
            <th>Person</th>
            <th>Was</th>
            <th>Now</th>
            <th>Last contact</th>
            <th>Last subject</th>
          </tr>
        </thead>
        <tbody>
          {people.map((p, i) => (
            <tr key={i}>
              <td>{i + 1}</td>
              <td>
                <PersonLink person={ref(p)} />
              </td>
              <td>{score(p.past_score)}</td>
              <td>{score(p.score)}</td>
              <td>{str(p.last_contact) ? shortDate(String(p.last_contact)) : '–'}</td>
              <td>{str(p.last_subject) ?? '–'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function names(list: Obj[], extra: (p: Obj) => string): ReactNode {
  if (!list.length) return 'none';
  return list.map((p, i) => (
    <span key={i}>
      {i > 0 && ', '}
      <PersonLink person={ref(p)} /> {extra(p)}
    </span>
  ));
}

/** Per-company totals: `counts` when present, else the list lengths. */
function counts(c: Obj): { current: number; alumni: number; one_hop: number } {
  const k = isObj(c.counts) ? c.counts : {};
  return {
    current: num(k.current) ?? arr(c.current).length,
    alumni: num(k.alumni) ?? arr(c.alumni).length,
    one_hop: num(k.one_hop) ?? arr(c.one_hop).length,
  };
}

function Accounts({ result }: { result: Obj }) {
  if (!Array.isArray(result.companies)) return null;
  const rows = result.companies.filter(isObj);
  if (!rows.length) return <p className="muted">No companies in the result.</p>;
  return (
    <>
      <div className="table-wrap">
        <table aria-label="Account coverage">
          <thead>
            <tr>
              <th>Company</th>
              <th>Current</th>
              <th>Alumni</th>
              <th>One hop</th>
              <th>Coverage</th>
              <th>Best contact</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((c, i) => {
              const current = arr(c.current).filter(isObj);
              const alumni = arr(c.alumni).filter(isObj);
              const n = counts(c);
              const best = [...current, ...alumni].find((p) => num(p.id) === num(c.best) && num(c.best) !== null);
              return (
                <tr key={i}>
                  <td>
                    {str(c.company) ?? str(c.domain) ?? '–'}
                    {str(c.domain) && str(c.company) ? <small> · {String(c.domain)}</small> : null}
                  </td>
                  <td>{n.current}</td>
                  <td>{n.alumni}</td>
                  <td>{n.one_hop}</td>
                  <td>
                    <span className={`tag ${c.coverage === 'none' ? 'neutral' : c.coverage === 'warm' ? '' : 'warn'}`}>{str(c.coverage) ?? '–'}</span>
                  </td>
                  <td>
                    {best ? (
                      <>
                        <PersonLink person={ref(best)} /> ({score(best.score)}
                        {alumni.includes(best) ? ', alumni' : ''})
                      </>
                    ) : num(c.best) !== null ? (
                      <PersonLink person={{ id: num(c.best), display_name: null }} />
                    ) : (
                      '–'
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {rows.map((c, i) => {
        const current = arr(c.current).filter(isObj);
        const alumni = arr(c.alumni).filter(isObj);
        const hop = arr(c.one_hop).filter(isObj);
        if (!current.length && !alumni.length && !hop.length) return null;
        const n = counts(c);
        // the engine caps each list at limit_per_company; `counts` has the totals
        const of = (shown: number, total: number) => (total > shown ? ` (showing ${shown} of ${total})` : '');
        const viaName = (id: number | null): string => {
          if (id === null) return '–';
          const tie = [...current, ...alumni].find((p) => num(p.id) === id);
          return (tie && str(tie.display_name)) || `#${id}`;
        };
        return (
          <details key={i} className="account-detail">
            <summary>{str(c.company) ?? str(c.domain) ?? 'Company'}: people</summary>
            <p>
              Current: {names(current, (p) => [str(p.title), `(${score(p.score)})`].filter(Boolean).join(' '))}
              {of(current.length, n.current)}
            </p>
            <p>
              Alumni: {names(alumni, (p) => `(alumni${str(p.left_seen) ? `, left ${String(p.left_seen)}` : ''})`)}
              {of(alumni.length, n.alumni)}
            </p>
            <p>
              One hop: {names(hop, (p) => `(via ${viaName(num(p.via))}, strength ${num(p.strength)?.toFixed(2) ?? '–'})`)}
              {of(hop.length, n.one_hop)}
            </p>
          </details>
        );
      })}
    </>
  );
}

function skippedLine(s: Partial<Skipped> & Obj): string {
  const who = num(s.person_id) !== null ? ` for #${String(s.person_id)}` : '';
  return `${str(s.op) ?? 'request'}${who}: ${String(s.reason ?? 'skipped').replace(/_/g, ' ')}`;
}

function Refresh({ result }: { result: Obj }) {
  if (!Array.isArray(result.updated) && !Array.isArray(result.not_found)) return null;
  const updated = arr(result.updated).filter(isObj);
  const notFound = arr(result.not_found);
  const skipped = arr(result.skipped).filter(isObj);
  const failed = arr(result.failed).filter(isObj);
  const unchanged = arr(result.unchanged).length;
  return (
    <>
      {updated.length ? (
        <div className="table-wrap">
          <table aria-label="Updated people">
            <thead>
              <tr>
                <th>Person</th>
                <th>New fields</th>
              </tr>
            </thead>
            <tbody>
              {updated.map((u, i) => (
                <tr key={i}>
                  <td>
                    <PersonLink person={ref(u)} />
                  </td>
                  <td>
                    {Object.entries(isObj(u.fields) ? u.fields : {})
                      .map(([k, v]) => `${k}: ${String(v)}`)
                      .join(' · ') || '–'}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="muted">No records updated.</p>
      )}
      {notFound.length > 0 && (
        <p>
          Not found:{' '}
          {notFound.map((x, i) => {
            const r = isObj(x) ? ref(x) : { id: num(x), display_name: null };
            return (
              <span key={i}>
                {i > 0 && ', '}
                <PersonLink person={r} />
              </span>
            );
          })}
        </p>
      )}
      {unchanged > 0 && (
        <p className="muted">
          {unchanged} {unchanged === 1 ? 'person' : 'people'} already up to date
        </p>
      )}
      {failed.length > 0 && (
        <p>
          Failed:{' '}
          {failed.map((f, i) => (
            <span key={i}>
              {i > 0 && '; '}
              <PersonLink person={ref(f)} /> ({str(f.error) ?? 'error'})
            </span>
          ))}
        </p>
      )}
      {skipped.length > 0 && (
        <ul className="notes-out">
          {skipped.map((s, i) => (
            <li key={i}>Skipped {skippedLine(s)}</li>
          ))}
        </ul>
      )}
    </>
  );
}

function Verify({ result }: { result: Obj }) {
  if (!('email' in result)) return null;
  const mutual = isObj(result.mutual) ? result.mutual : null;
  return (
    <div className="verify-card">
      {str(result.email) ? (
        <p>
          <code>{String(result.email)}</code> <span className="status-tag">{str(result.status) ?? 'unverified'}</span>
          {str(result.verified_at) && <small> · verified {shortDate(String(result.verified_at))}</small>}
        </p>
      ) : (
        <p className="muted">No email to verify.</p>
      )}
      {mutual ? (
        <p>
          Freshest mutual who replied: <PersonLink person={ref(mutual)} />
          {str(mutual.last_reply) ? ` · last reply ${shortDate(String(mutual.last_reply))}` : ''}
          {str(mutual.last_subject) ? ` · “${String(mutual.last_subject)}”` : ''}
        </p>
      ) : (
        <p className="muted">No mutual who replied.</p>
      )}
    </div>
  );
}

function Watch({ result }: { result: Obj }) {
  if (!Array.isArray(result.moves)) return null;
  const moves = result.moves.filter(isObj);
  const hits = moves.filter((m) => m.target_hit === true).length;
  const titles = arr(result.title_changes).filter(isObj);
  const added = arr(result.new_connections).filter(isObj);
  const removed = arr(result.removed_connections).filter(isObj);
  const src = isObj(result.sources) ? result.sources : null;
  const exportName = (e: unknown) =>
    isObj(e) ? `${str(e.label) ?? `source #${String(e.id)}`}${str(e.imported_at) ? ` (${shortDate(String(e.imported_at))})` : ''}` : '–';
  const role = (title: unknown, company: unknown) => [str(title), str(company)].filter(Boolean).join(' · ') || '–';
  return (
    <>
      {src && (
        <p className="muted">
          Comparing {exportName(src.previous)} with {exportName(src.latest)}
        </p>
      )}
      <p>
        {moves.length} move{moves.length === 1 ? '' : 's'} · {hits} move{hits === 1 ? '' : 's'} into your target companies
        {num(result.checked) !== null ? ` · ${String(result.checked)} profiles checked` : ''}
      </p>
      {moves.length > 0 && (
        <div className="table-wrap">
          <table aria-label="Job moves">
            <thead>
              <tr>
                <th>Person</th>
                <th>From</th>
                <th>To</th>
                <th>Detected by</th>
                <th>Target</th>
              </tr>
            </thead>
            <tbody>
              {moves.map((m, i) => (
                <tr key={i} className={m.target_hit === true ? 'hit' : undefined}>
                  <td>
                    <PersonLink person={ref(m)} />
                  </td>
                  <td>{role(m.from_title, m.from)}</td>
                  <td>{role(m.to_title, m.to)}</td>
                  <td>{String(m.detected_by ?? '–').replace(/_/g, ' ')}</td>
                  <td>{m.target_hit === true ? <span className="tag warn">target</span> : ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {titles.length > 0 && (
        <div className="table-wrap">
          <table aria-label="Title changes">
            <thead>
              <tr>
                <th>Person</th>
                <th>Company</th>
                <th>From</th>
                <th>To</th>
              </tr>
            </thead>
            <tbody>
              {titles.map((t, i) => (
                <tr key={i}>
                  <td>
                    <PersonLink person={ref(t)} />
                  </td>
                  <td>{str(t.company) ?? '–'}</td>
                  <td>{str(t.from) ?? '–'}</td>
                  <td>{str(t.to) ?? '–'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {added.length > 0 && <p>New connections: {names(added, (p) => `(${role(p.title, p.company)})`)}</p>}
      {removed.length > 0 && <p>Removed connections: {names(removed, (p) => `(${role(p.title, p.company)})`)}</p>}
    </>
  );
}

const RENDERERS: Record<string, (p: { result: Obj }) => ReactNode> = {
  brief: Brief,
  path: PathResult,
  reconnect: Reconnect,
  accounts: Accounts,
  refresh: Refresh,
  verify: Verify,
  watch: Watch,
};

/** Whether `result` has a human renderer for this playbook. */
export function hasRenderer(playbook: string, result: unknown): boolean {
  if (!isObj(result)) return false;
  switch (playbook) {
    case 'brief':
      return typeof result.markdown === 'string';
    case 'path':
      return Array.isArray(result.paths);
    case 'reconnect':
      return Array.isArray(result.people);
    case 'accounts':
      return Array.isArray(result.companies);
    case 'refresh':
      return Array.isArray(result.updated) || Array.isArray(result.not_found);
    case 'verify':
      return 'email' in result;
    case 'watch':
      return Array.isArray(result.moves);
    default:
      return false;
  }
}

/** The result of one playbook run, with the raw JSON behind a toggle. */
export function PlaybookResult({ playbook, result }: { playbook: string; result: unknown }) {
  if (result === null || result === undefined) return null;
  const Render = RENDERERS[playbook];
  const human = Render && hasRenderer(playbook, result) ? <Render result={result as Obj} /> : null;
  return (
    <section className="playbook-result" aria-label="Result">
      {human && (
        <>
          <h3 className="subtle-label">Result</h3>
          {human}
        </>
      )}
      <details open={!human} className="raw-json">
        <summary className="subtle-label">Raw JSON</summary>
        <pre className="result-json">{JSON.stringify(result, null, 2)}</pre>
      </details>
    </section>
  );
}
