import { useState } from 'react';
import type { EditOp, PersonDetail, ScalarField } from '../api/types';
import { useApp } from '../app/context';
import { href } from '../app/router';
import { Avatar, EvidenceList, ErrorNote, Loading, PersonFlags, ScoreBar } from '../components/bits';
import { Icon } from '../components/Icon';
import { displayName, shortDate, sourceLabel } from '../lib/format';
import { useLoad } from '../lib/useLoad';
import { useShortlist } from '../lib/useShortlist';

const FIELDS: [ScalarField, string][] = [
  ['display_name', 'Name'],
  ['title', 'Title'],
  ['company', 'Company'],
  ['location', 'Location'],
  ['bio', 'Bio'],
];

const COMPONENTS = ['recency', 'frequency', 'reciprocity', 'meetings', 'breadth'] as const;

export function PersonPage({ id }: { id: string }) {
  const { api, toast, refreshStats } = useApp();
  const detail = useLoad(() => api.person(id), [api, id]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function apply(ops: EditOp[], done: string) {
    setBusy(true);
    setError(null);
    try {
      await api.edits(ops);
      toast(done);
      detail.reload();
      refreshStats();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (detail.error) {
    return (
      <>
        <BackLink />
        <ErrorNote error={detail.error} />
      </>
    );
  }
  if (!detail.data) return <Loading what="Loading person" />;
  const d = detail.data;
  const p = d.person;
  const name = displayName(p);

  return (
    <>
      <BackLink />
      <div className="person-title">
        <Avatar name={name} />
        <div>
          <h1>{name}</h1>
          <div className="person-role">{[p.title, p.company].filter(Boolean).join(' · ') || 'Title and company unknown'}</div>
          <div className="record-signals">
            <ScoreBar score={p.score} />
            <PersonFlags person={p} />
            {d.merge.flagged && (
              <a className="tag warn" href="#/review">
                Flagged merge · review
              </a>
            )}
          </div>
        </div>
      </div>
      {d.resolved_from !== undefined && (
        <div className="alert info">
          #{d.resolved_from} was merged into this person. Links to the old id keep working.
        </div>
      )}
      {error && (
        <div className="alert" role="alert">
          {error}
        </div>
      )}

      <div className="person-page" aria-busy={busy}>
        <div>
          <section className="panel" aria-label="Fields">
            <h2>
              What the record says <small>each edit is an override; imports never overwrite it</small>
            </h2>
            {FIELDS.map(([field, label]) => (
              <FieldRow key={field} detail={d} field={field} label={label} onOps={apply} />
            ))}
          </section>

          <ScorePanel detail={d} />

          <TagsNotes detail={d} onOps={apply} />

          <Identities detail={d} onOps={apply} />

          <section className="panel" aria-label="Recent interactions">
            <h2>
              Recent interactions <small>subjects and titles only, never message bodies</small>
            </h2>
            <EvidenceList items={d.recent_interactions} empty="No interactions imported for this person." />
          </section>
        </div>

        <div>
          <section className="panel" aria-label="Privacy">
            <h2>Privacy</h2>
            <div className="privacy-fields">
              <label className="check">
                <input
                  type="checkbox"
                  checked={p.do_not_share}
                  onChange={(e) =>
                    void apply(
                      [{ op: 'flag', person: p.id, field: 'do_not_share', value: e.target.checked }],
                      e.target.checked ? 'Excluded from views and exports.' : 'Can appear in views again.',
                    )
                  }
                />
                Do not share · exclude from views, exports and MCP over HTTP
              </label>
              <label className="check">
                <input
                  type="checkbox"
                  checked={p.no_enrich}
                  onChange={(e) =>
                    void apply(
                      [{ op: 'flag', person: p.id, field: 'no_enrich', value: e.target.checked }],
                      e.target.checked ? 'Never sent to an enricher.' : 'Enrichment allowed again (still quote-gated).',
                    )
                  }
                />
                Do not enrich · never sent to an enrichment provider
              </label>
            </div>
          </section>

          <HowConnected id={p.id} />

          <section className="panel" aria-label="Mutuals">
            <h2>Mutuals</h2>
            {d.mutuals.length ? (
              d.mutuals.map((m) => (
                <div className="mutual-row" key={m.person.id}>
                  <div>
                    <a href={href({ name: 'person', id: String(m.person.id) })}>{displayName(m.person)}</a>
                    <small>
                      {m.n_shared} shared {m.tie_kind} · {[m.person.title, m.person.company].filter(Boolean).join(' · ')}
                    </small>
                  </div>
                  <span className="strength" title="tie weight">
                    {m.weight.toFixed(2)}
                  </span>
                </div>
              ))
            ) : (
              <p>No shared threads or meetings with other people yet.</p>
            )}
          </section>

          <SideActions detail={d} />
        </div>
      </div>
    </>
  );
}

function BackLink() {
  return (
    <a className="back-link" href="#/people">
      <Icon name="arrow" /> People
    </a>
  );
}

function FieldRow({
  detail,
  field,
  label,
  onOps,
}: {
  detail: PersonDetail;
  field: ScalarField;
  label: string;
  onOps: (ops: EditOp[], done: string) => Promise<void>;
}) {
  const p = detail.person;
  const value = p[field];
  const source = p.field_sources?.[field] ?? null;
  const suggested = detail.suggested?.[field];
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value ?? '');
  const lower = label.toLowerCase();

  return (
    <div className="field-row" role="group" aria-label={label}>
      <span>{label}</span>
      {editing ? (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            const v = draft.trim();
            setEditing(false);
            if (v === (value ?? '')) return;
            void onOps(
              v ? [{ op: 'set', person: p.id, field, value: v }] : [{ op: 'unset', person: p.id, field }],
              `${label} saved as your edit.`,
            );
          }}
        >
          <input aria-label={label} value={draft} autoFocus onChange={(e) => setDraft(e.target.value)} maxLength={1000} />
          <button type="submit" className="btn primary">
            Save
          </button>
          <button type="button" className="btn" onClick={() => setEditing(false)}>
            Cancel
          </button>
        </form>
      ) : (
        <>
          <div className="value">
            {value ? value : <span className="unknown">unknown</span>}
            <small>
              {source === 'override' ? <span className="tag">your edit</span> : null}
              {source && source !== 'override' ? `from ${sourceLabel(source)}` : null}
            </small>
            {suggested && suggested.value !== value && (
              <span className="suggested">
                Suggested: {suggested.value} · {suggested.from} {shortDate(suggested.at)}
                <button
                  type="button"
                  className="btn"
                  aria-label={`Use suggested ${lower}`}
                  onClick={() =>
                    void onOps([{ op: 'set', person: p.id, field, value: suggested.value }], `${label} set from the suggestion.`)
                  }
                >
                  Use
                </button>
              </span>
            )}
          </div>
          <div className="row-actions">
            <button
              type="button"
              className="btn"
              aria-label={`Edit ${lower}`}
              onClick={() => {
                setDraft(value ?? '');
                setEditing(true);
              }}
            >
              <Icon name="edit" />
              Edit
            </button>
            {value && (
              <button
                type="button"
                className="btn text"
                aria-label={`Clear ${lower}`}
                onClick={() => void onOps([{ op: 'unset', person: p.id, field }], `${label} cleared.`)}
              >
                Clear
              </button>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function ScorePanel({ detail }: { detail: PersonDetail }) {
  const p = detail.person;
  const c = p.components;
  const raw = c.raw;
  return (
    <section className="score-detail" aria-label="Relationship score">
      <div className="score-detail-title">
        <div>
          <span className="eyebrow">EXPLAINED RELATIONSHIP SCORE</span>
          <h3>
            {Math.round(p.score * 100)}
            <small> /100</small>
          </h3>
        </div>
        <span className="tag">
          {c.preset} preset · half-life {c.half_life_days} days
        </span>
      </div>
      {COMPONENTS.map((k) => (
        <div className="score-line" key={k}>
          <span>{k}</span>
          <div>
            <i style={{ width: `${Math.round(c[k] * 100)}%` }} />
          </div>
          <b>{c[k].toFixed(3)}</b>
          <small>× {c.weights[k].toFixed(2)}</small>
        </div>
      ))}
      {raw && (
        <p className="raw-counts">
          {raw.n_interactions} interactions · {raw.replies_from_them} replies / {raw.messages_from_you} sent ·{' '}
          {raw.n_meetings} meetings · {raw.channels.length} channels ({raw.channels.join(', ')}) ·{' '}
          {raw.days_since_last === null ? 'no contact date' : `${Math.round(raw.days_since_last)} days since last contact`}
        </p>
      )}
      <p>
        score = Σ weight × component. First seen {shortDate(p.first_seen)}, last contact {shortDate(p.last_contact)}. A
        higher score does not guarantee an introduction.
      </p>
    </section>
  );
}

function TagsNotes({ detail, onOps }: { detail: PersonDetail; onOps: (ops: EditOp[], done: string) => Promise<void> }) {
  const p = detail.person;
  const [tag, setTag] = useState('');
  const [note, setNote] = useState('');
  return (
    <section className="panel" aria-label="Tags and notes">
      <h2>
        Tags and notes <small>yours only; notes are never sent anywhere</small>
      </h2>
      <div className="tag-list">
        {p.tags.length ? (
          p.tags.map((t) => (
            <span className="tag" key={t}>
              {t}
              <button
                type="button"
                aria-label={`Remove tag ${t}`}
                onClick={() => void onOps([{ op: 'untag', person: p.id, value: t }], `Tag “${t}” removed.`)}
              >
                ×
              </button>
            </span>
          ))
        ) : (
          <small className="muted">No tags yet.</small>
        )}
      </div>
      <form
        className="tag-form"
        onSubmit={(e) => {
          e.preventDefault();
          const v = tag.trim();
          if (!v) return;
          setTag('');
          void onOps([{ op: 'tag', person: p.id, value: v }], `Tagged “${v}”.`);
        }}
      >
        <input className="inline-input" aria-label="New tag" value={tag} onChange={(e) => setTag(e.target.value)} placeholder="investor, devtools…" maxLength={100} />
        <button type="submit" className="btn">
          <Icon name="plus" />
          Add tag
        </button>
      </form>
      <ul className="notes-list">
        {detail.notes.map((n, i) => (
          <li key={i}>
            {n.value}
            <small>{shortDate(n.set_at)}</small>
          </li>
        ))}
      </ul>
      <form
        className="note-form"
        onSubmit={(e) => {
          e.preventDefault();
          const v = note.trim();
          if (!v) return;
          setNote('');
          void onOps([{ op: 'note', person: p.id, value: v }], 'Note added.');
        }}
      >
        <textarea aria-label="New note" value={note} onChange={(e) => setNote(e.target.value)} placeholder="How you met, what they care about…" maxLength={4000} />
        <button type="submit" className="btn">
          <Icon name="plus" />
          Add note
        </button>
      </form>
    </section>
  );
}

function Identities({ detail, onOps }: { detail: PersonDetail; onOps: (ops: EditOp[], done: string) => Promise<void> }) {
  const [selected, setSelected] = useState<number[]>([]);
  const p = detail.person;
  const toggle = (id: number) => setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  return (
    <section className="panel" aria-label="Identities">
      <h2>
        Identities <small>every handle that resolved to this person</small>
      </h2>
      <div className="table-wrap">
        <table className="ident-table">
          <thead>
            <tr>
              <th>
                <span className="sr-only">Select</span>
              </th>
              <th>Kind</th>
              <th>Value</th>
              <th>Confidence</th>
              <th>Source</th>
            </tr>
          </thead>
          <tbody>
            {detail.identities.map((i) => (
              <tr key={i.id}>
                <td>
                  <input
                    type="checkbox"
                    aria-label={`Select ${i.value}`}
                    checked={selected.includes(i.id)}
                    onChange={() => toggle(i.id)}
                  />
                </td>
                <td>{i.kind}</td>
                <td>
                  {i.value}
                  {i.verification && (
                    <div className="verified">
                      {i.verification.status} · {i.verification.via} {shortDate(i.verification.at)}
                    </div>
                  )}
                </td>
                <td>
                  {i.confidence === null ? '–' : i.confidence.toFixed(2)}
                  {i.match_rule ? ` · ${i.match_rule}` : ''}
                </td>
                <td>{i.source.label ?? i.source.kind}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="modal-actions">
        {detail.merge.merged_from.length > 0 && (
          <small className="muted">Merged from {detail.merge.merged_from.map((x) => `#${x}`).join(', ')}</small>
        )}
        <button
          type="button"
          className="btn"
          disabled={!selected.length || selected.length >= detail.identities.length}
          onClick={() => {
            const ids = selected;
            setSelected([]);
            void onOps([{ op: 'split', person: p.id, identities: ids }], `Split ${ids.length} ${ids.length === 1 ? 'identity' : 'identities'} into a separate person.`);
          }}
        >
          Split {selected.length || ''} {selected.length === 1 ? 'identity' : 'identities'} off
        </button>
      </div>
    </section>
  );
}

function HowConnected({ id }: { id: number }) {
  const { api } = useApp();
  const how = useLoad(() => api.connections(id, 3), [api, id]);
  return (
    <section className="panel" aria-label="How connected">
      <h2>How you’re connected</h2>
      {how.loading && <Loading what="Finding paths" />}
      <ErrorNote error={how.error} />
      {how.data && (
        <>
          <p>
            {how.data.direct
              ? `Direct: score ${how.data.direct.score.toFixed(2)}, last contact ${shortDate(how.data.direct.last_contact)}.`
              : 'No direct interaction on record.'}
          </p>
          {how.data.paths.map((path) => (
            <div className="path-row" key={path.via.id}>
              <div>
                You → <a href={href({ name: 'person', id: String(path.via.id) })}>{displayName(path.via)}</a> →{' '}
                {displayName(how.data!.target)}
                <small>
                  {path.reason.replace('_', ' ')} · your score {path.my_score.toFixed(2)} × link {path.link_weight.toFixed(2)}
                </small>
                <EvidenceList items={path.evidence} />
              </div>
              <span className="strength" title="strength = my score × link weight">
                {path.strength.toFixed(3)}
              </span>
            </div>
          ))}
          {!how.data.paths.length && <p>No mutual paths found in your graph.</p>}
          <p className="path-warning">A path is a lead, not a promise: ask the connector before you ask for the intro.</p>
        </>
      )}
    </section>
  );
}

function SideActions({ detail }: { detail: PersonDetail }) {
  const { api, toast, refreshStats } = useApp();
  const shortlist = useShortlist();
  const p = detail.person;
  const saved = shortlist.ids.has(p.id);
  return (
    <section className="panel side-actions" aria-label="Next steps">
      <button type="button" className="btn" aria-pressed={saved} onClick={() => void shortlist.toggle(p)}>
        <Icon name="bookmark" />
        {saved ? 'On your shortlist' : 'Add to shortlist'}
      </button>
      <button
        type="button"
        className="btn"
        onClick={async () => {
          try {
            await api.createIntro({ person: p.id, stage: 'draft' });
            toast('Added to your introductions board. Nothing was sent.');
            refreshStats();
          } catch (e) {
            toast((e as Error).message);
          }
        }}
      >
        <Icon name="mail" />
        Track an introduction
      </button>
      <a className="btn primary" href={`#/playbooks/brief?person=${p.id}`}>
        <Icon name="route" />
        Brief before you reach out
      </a>
    </section>
  );
}
