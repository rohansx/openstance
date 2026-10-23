import { useState } from 'react';
import type { EditOp, FlaggedMerge, Identity } from '../api/types';
import { useApp } from '../app/context';
import { href } from '../app/router';
import { Empty, ErrorNote, Loading, PageHead } from '../components/bits';
import { Icon } from '../components/Icon';
import { displayName, pct } from '../lib/format';
import { useLoad } from '../lib/useLoad';

const BULK_THRESHOLD = 0.85;

function IdentList({ items }: { items: Identity[] }) {
  return (
    <ul>
      {items.map((i) => (
        <li key={i.id}>
          {i.kind}: {i.value} <span className="muted">({i.source.label ?? i.source.kind})</span>
        </li>
      ))}
    </ul>
  );
}

export function Review() {
  const { api, toast, refreshStats } = useApp();
  const queue = useLoad(() => api.review(), [api]);
  const [selected, setSelected] = useState<number[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function apply(ops: EditOp[], done: string) {
    if (!ops.length) return;
    setBusy(true);
    setError(null);
    try {
      await api.edits(ops);
      toast(done);
      setSelected([]);
      queue.reload();
      refreshStats();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const flagged = queue.data?.flagged ?? [];
  const suggestions = queue.data?.suggestions ?? [];
  const confirmOps = (rows: FlaggedMerge[]): EditOp[] => rows.map((f) => ({ op: 'confirm', person: f.person.id }));
  const aboveThreshold = flagged.filter((f) => f.confidence >= BULK_THRESHOLD - 1e-9);

  return (
    <>
      <PageHead
        eyebrow="KEEP THE RECORD HONEST"
        title="Same person, or not?"
        sub="Merges the resolver was unsure about, with the rule and evidence behind each. Confirm or split; nothing is merged silently."
      />
      <ErrorNote error={queue.error} />
      {error && (
        <div className="alert" role="alert">
          {error}
        </div>
      )}
      {queue.loading && !queue.data && <Loading what="Loading the review queue" />}
      {queue.data && !flagged.length && !suggestions.length && (
        <Empty title="Nothing to review." icon="check">
          Every merge in your graph was made by an exact rule or confirmed by you.
        </Empty>
      )}

      {flagged.length > 0 && (
        <section aria-label="Flagged merges">
          <h3 className="subtle-label">Flagged merges · {flagged.length}</h3>
          <div className="bulk-bar">
            <button
              type="button"
              className="btn primary"
              disabled={!selected.length || busy}
              onClick={() =>
                void apply(
                  confirmOps(flagged.filter((f) => selected.includes(f.person.id))),
                  `Confirmed ${selected.length} merges.`,
                )
              }
            >
              <Icon name="check" />
              Confirm {selected.length} selected
            </button>
            <button
              type="button"
              className="btn"
              disabled={!aboveThreshold.length || busy}
              onClick={() => void apply(confirmOps(aboveThreshold), `Confirmed ${aboveThreshold.length} merges at or above 85%.`)}
            >
              Confirm all at or above {pct(BULK_THRESHOLD)}% ({aboveThreshold.length})
            </button>
          </div>
          {flagged.map((f) => {
            const name = displayName(f.person);
            const on = selected.includes(f.person.id);
            return (
              <article className="review-card" key={f.person.id}>
                <div className="review-head">
                  <input
                    type="checkbox"
                    className="check"
                    aria-label={`Select ${name}`}
                    checked={on}
                    onChange={() =>
                      setSelected((s) => (on ? s.filter((x) => x !== f.person.id) : [...s, f.person.id]))
                    }
                  />
                  <h3>
                    <a href={href({ name: 'person', id: String(f.person.id) })}>{name}</a>
                  </h3>
                  <span className="confidence" title="rule confidence">
                    {pct(f.confidence)}%
                  </span>
                  <span className="tag neutral">rule: {f.rule}</span>
                </div>
                <p className="why">{f.why}</p>
                <div className="ident-compare">
                  <div>
                    <h4>Attached by the flagged rule</h4>
                    <IdentList items={f.flagged_identities} />
                  </div>
                  <div>
                    <h4>The rest of this person</h4>
                    <IdentList items={f.other_identities} />
                  </div>
                </div>
                <div className="row-actions">
                  <button
                    type="button"
                    className="btn primary"
                    disabled={busy}
                    aria-label={`Confirm merge for ${name}`}
                    onClick={() => void apply([{ op: 'confirm', person: f.person.id }], `Confirmed: ${name} is one person.`)}
                  >
                    <Icon name="check" />
                    Confirm
                  </button>
                  <button
                    type="button"
                    className="btn"
                    disabled={busy}
                    aria-label={`Split flagged identities off ${name}`}
                    onClick={() =>
                      void apply(
                        [{ op: 'split', person: f.person.id, identities: f.flagged_identities.map((i) => i.id) }],
                        `Split: the flagged identities are now a separate person.`,
                      )
                    }
                  >
                    Split
                  </button>
                </div>
              </article>
            );
          })}
        </section>
      )}

      {suggestions.length > 0 && (
        <section aria-label="Suggestions">
          <h3 className="subtle-label">Suggestions · never merged automatically</h3>
          {suggestions.map((s) => (
            <article className="review-card" key={`${s.a.id}-${s.b.id}`}>
              <div className="review-head">
                <span>
                  <a href={href({ name: 'person', id: String(s.a.id) })}>{displayName(s.a)}</a>
                  <small className="muted"> {[s.a.title, s.a.company].filter(Boolean).join(' · ')}</small>
                </span>
                <span aria-hidden="true">↔</span>
                <span>
                  <a href={href({ name: 'person', id: String(s.b.id) })}>{displayName(s.b)}</a>
                  <small className="muted"> {[s.b.title, s.b.company].filter(Boolean).join(' · ')}</small>
                </span>
                <span className="confidence">{pct(s.confidence)}%</span>
                <span className="tag neutral">rule: {s.rule}</span>
              </div>
              <p className="why">{s.why}</p>
              <div className="row-actions">
                <button
                  type="button"
                  className="btn"
                  disabled={busy}
                  aria-label={`Merge ${displayName(s.a)} and ${displayName(s.b)}`}
                  onClick={() => void apply([{ op: 'merge', a: s.a.id, b: s.b.id }], 'Merged.')}
                >
                  Merge
                </button>
                <button
                  type="button"
                  className="btn text"
                  disabled={busy}
                  onClick={() => void apply([{ op: 'reject', a: s.a.id, b: s.b.id }], 'Kept apart. This pair will not be suggested again.')}
                >
                  Keep apart
                </button>
              </div>
            </article>
          ))}
        </section>
      )}

      {(queue.data?.info.length ?? 0) > 0 && (
        <section className="panel" aria-label="Notes from the resolver">
          <h2>Notes from the resolver</h2>
          {queue.data!.info.map((i, n) => (
            <div className="source-row" key={n}>
              <span>{i.message}</span>
              <span className="tag neutral">{i.kind}</span>
            </div>
          ))}
        </section>
      )}
    </>
  );
}
