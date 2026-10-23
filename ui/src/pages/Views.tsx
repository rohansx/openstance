import { useState } from 'react';
import { chipsFromQuery, emptyQuery } from '../api/query';
import type { Query, ViewSearchResult, ViewSummary } from '../api/types';
import { useApp } from '../app/context';
import { Empty, ErrorNote, Loading, PageHead } from '../components/bits';
import { Dialog } from '../components/Dialog';
import { Icon } from '../components/Icon';
import { downloadBlob } from '../lib/download';
import { shortDate } from '../lib/format';
import { useLoad } from '../lib/useLoad';

function describeQuery(q: ViewSummary['query']): string {
  const query = { ...emptyQuery(), ...(q as Partial<Query>) } as Query;
  const parts = [query.text ? `“${query.text}”` : null, ...chipsFromQuery(query).map((c) => c.label)].filter(Boolean);
  return parts.length ? parts.join(' · ') : 'Everyone (no filters)';
}

export function Views() {
  const { api, toast } = useApp();
  const views = useLoad(() => api.views('all'), [api]);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function importFile(file: File | undefined) {
    if (!file) return;
    setError(null);
    try {
      const v = await api.importView(file);
      toast(`Imported “${v.name}” · ${v.rows} people.`);
      views.reload();
    } catch (e) {
      setError((e as Error).message);
    }
  }

  const list = views.data ?? [];
  return (
    <>
      <PageHead
        eyebrow="SHARE SLICES. KEEP THE GRAPH."
        title="Small views. Clear boundaries."
        sub="A view is a saved query, exported as an encrypted .ost file for the people you choose. Import views others share with you and search them locally."
        aside={
          <button type="button" className="btn primary" onClick={() => setCreating(true)}>
            <Icon name="plus" />
            New view
          </button>
        }
      />
      <div className="view-notice">
        <Icon name="shield" />
        <span>
          <strong>Nothing is shared automatically.</strong> Exports are encrypted to the recipients’ age keys and always
          leave out people marked “Do not share”. A file someone already received cannot be recalled; expiry is enforced
          by openstance on their side.
        </span>
      </div>
      <ErrorNote error={views.error} />
      {error && (
        <div className="alert" role="alert">
          {error}
        </div>
      )}
      {views.loading && !views.data && <Loading what="Loading views" />}
      {views.data && !list.length && (
        <Empty title="A useful search is worth keeping." icon="bookmark">
          Search your network, then choose “Save view”, or create one here from a query.
        </Empty>
      )}
      {list.length > 0 && (
        <div className="section-grid">
          {list.map((v) => (
            <ViewCard key={`${v.direction}:${v.name}`} view={v} />
          ))}
        </div>
      )}
      <section className="panel" style={{ marginTop: 24 }}>
        <h2>Import a view someone shared</h2>
        <p>Imported views are decrypted with your key and searched locally. They never merge into your own graph.</p>
        <label className="btn">
          <Icon name="upload" />
          Import a .ost file
          <input
            className="sr-only"
            type="file"
            accept=".ost,application/octet-stream"
            onChange={(e) => {
              void importFile(e.target.files?.[0]);
              e.target.value = '';
            }}
          />
        </label>
      </section>
      <section className="panel" style={{ marginTop: 24 }}>
        <h2>The sharing contract</h2>
        <div className="contract-grid">
          <div>
            <strong>01 · Explicit scope</strong>
            <p>Only people matching the saved query are in a view, with professional fields only: no notes, no subjects.</p>
          </div>
          <div>
            <strong>02 · Person-level exclusions</strong>
            <p>“Do not share” always removes a person from every view export.</p>
          </div>
          <div>
            <strong>03 · Honest limits</strong>
            <p>An offline file cannot be recalled once received. Choose recipients and expiry with that in mind.</p>
          </div>
        </div>
      </section>
      {creating && (
        <CreateViewDialog
          onClose={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            views.reload();
          }}
        />
      )}
    </>
  );
}

function ViewCard({ view: v }: { view: ViewSummary }) {
  const { api, toast } = useApp();
  const [q, setQ] = useState('');
  const [results, setResults] = useState<ViewSearchResult[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const own = v.direction === 'own';
  const [now] = useState(() => Date.now());
  const expired = v.expires_at !== null && new Date(v.expires_at).getTime() < now;

  return (
    <article className="panel saved-view">
      <div className="saved-view-icon">
        <Icon name="bookmark" />
        <span>{own ? 'YOUR VIEW' : `IMPORTED${v.owner.label ? ` · FROM ${v.owner.label.toUpperCase()}` : ''}`}</span>
      </div>
      <h2>{v.name}</h2>
      <p className="query-code">{describeQuery(v.query)}</p>
      <div className="view-counts">
        <b>{v.rows}</b>
        <span>people</span>
      </div>
      <p>
        {own ? 'Your view' : `Imported${v.owner.label ? ` from ${v.owner.label}` : ''}`} · created {shortDate(v.created_at)} ·{' '}
        {v.expires_at ? `${expired ? 'expired' : 'expires'} ${shortDate(v.expires_at)}` : 'never expires'}
        {own && v.excluded_do_not_share ? ` · ${v.excluded_do_not_share} left out: do not share` : ''}
        {own && v.recipients?.length ? ` · ${v.recipients.length} recipient${v.recipients.length === 1 ? '' : 's'}` : ''}
      </p>
      {error && <div className="alert">{error}</div>}
      <div className="view-actions">
        {own ? (
          <button
            type="button"
            className="btn primary"
            onClick={async () => {
              setError(null);
              try {
                const blob = await api.exportView(v.name);
                downloadBlob(blob, `${v.name}.ost`);
                toast(`Exported ${v.name}.ost, encrypted to its recipients.`);
              } catch (e) {
                setError((e as Error).message);
              }
            }}
          >
            <Icon name="down" />
            Export .ost
          </button>
        ) : (
          <form
            className="add-filter"
            onSubmit={async (e) => {
              e.preventDefault();
              setError(null);
              try {
                const r = await api.searchView(v.name, { q: q.trim() || undefined, limit: 20 });
                setResults(r.results);
              } catch (err) {
                setError((err as Error).message);
              }
            }}
          >
            <input type="search" aria-label={`Search ${v.name}`} value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search this view" />
            <button type="submit" className="btn">
              <Icon name="search" />
              Search
            </button>
          </form>
        )}
      </div>
      {results && (
        <div className="view-results">
          {results.length ? (
            results.map((r, i) => (
              <div className="source-row" key={i}>
                <div>
                  {r.person.display_name ?? 'Unnamed'}
                  <small>
                    {[r.person.title, r.person.company, r.person.location].filter(Boolean).join(' · ')}
                    {r.person.last_contact_month ? ` · last contact ${r.person.last_contact_month}` : ''}
                  </small>
                </div>
                <span className="tag">{r.person.score_bucket}</span>
              </div>
            ))
          ) : (
            <p>No matches in this view.</p>
          )}
        </div>
      )}
    </article>
  );
}

function CreateViewDialog({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const { api, toast } = useApp();
  const [name, setName] = useState('');
  const [query, setQuery] = useState('');
  const [ttl, setTtl] = useState('30d');
  const [to, setTo] = useState('self');
  const [error, setError] = useState<string | null>(null);
  return (
    <Dialog title="Create a view" onClose={onClose}>
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          setError(null);
          try {
            const v = await api.createView({
              name: name.trim(),
              query: query.trim(),
              ttl,
              to: to.split(/[\s,]+/).filter(Boolean),
            });
            toast(`Saved view “${v.name}” · ${v.rows} people.`);
            onCreated();
          } catch (err) {
            setError((err as Error).message);
          }
        }}
      >
        <label className="field">
          View name
          <input value={name} onChange={(e) => setName(e.target.value)} required maxLength={200} placeholder="devtools-investors" />
        </label>
        <label className="field">
          Query
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="devtools investor" />
        </label>
        <div className="form-grid">
          <label className="field">
            Expires after
            <select value={ttl} onChange={(e) => setTtl(e.target.value)}>
              <option value="7d">7 days</option>
              <option value="30d">30 days</option>
              <option value="90d">90 days</option>
              <option value="never">Never</option>
            </select>
          </label>
          <label className="field">
            Recipients (age public keys, or self)
            <input value={to} onChange={(e) => setTo(e.target.value)} required spellCheck={false} />
          </label>
        </div>
        {error && <div className="alert">{error}</div>}
        <div className="modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn primary">
            Save view <Icon name="bookmark" />
          </button>
        </div>
      </form>
    </Dialog>
  );
}
