import { useState } from 'react';
import { chipsFromQuery, queryToRequest } from '../api/query';
import type { Query } from '../api/types';
import { useApp } from '../app/context';
import { Dialog } from './Dialog';
import { Icon } from './Icon';

/** POST /views: a named, encrypted slice of the graph defined by a query. */
export function SaveViewDialog({ query, onClose, onSaved }: { query: Query; onClose: () => void; onSaved: () => void }) {
  const { api, toast } = useApp();
  const [name, setName] = useState('');
  const [ttl, setTtl] = useState('30d');
  const [to, setTo] = useState('self');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const chips = chipsFromQuery(query);

  return (
    <Dialog title="Save a slice of your network" onClose={onClose}>
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          if (!name.trim()) return;
          setBusy(true);
          setError(null);
          try {
            // A view stores the Query itself; `parse`/`explain` only steer a search.
            const { parse: _parse, explain: _explain, ...viewQuery } = queryToRequest(query);
            const v = await api.createView({
              name: name.trim(),
              query: viewQuery,
              ttl,
              to: to
                .split(/[\s,]+/)
                .map((s) => s.trim())
                .filter(Boolean),
            });
            toast(`Saved view “${v.name}” · ${v.rows} people.`);
            onSaved();
          } catch (err) {
            setError((err as Error).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        <label className="field">
          View name
          <input value={name} onChange={(e) => setName(e.target.value)} required maxLength={200} placeholder="e.g. devtools-investors" />
        </label>
        <div className="evidence-block">
          <h3>The saved query</h3>
          <p>{query.text ? `“${query.text}”` : 'No text'}</p>
          <p>{chips.length ? chips.map((c) => c.label).join(' · ') : 'No filters'}</p>
        </div>
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
        <p>
          A view saves the query, not a copy of your graph. Exports are encrypted <code>.ost</code> files and always exclude
          people marked “Do not share”. A file someone already received cannot be recalled.
        </p>
        {error && <div className="alert">{error}</div>}
        <div className="modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={busy}>
            Save view <Icon name="bookmark" />
          </button>
        </div>
      </form>
    </Dialog>
  );
}
