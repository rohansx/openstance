import { useState } from 'react';
import type { ImportResponse, SourceRow } from '../api/types';
import { dbPath, useApp } from '../app/context';
import { Empty, ErrorNote, Loading, PageHead } from '../components/bits';
import { Icon } from '../components/Icon';
import { shortDate } from '../lib/format';
import { useLoad } from '../lib/useLoad';

const n = (x: unknown) => (typeof x === 'number' ? x.toLocaleString('en-US') : '–');

// Importers that exist in openstance-import today.
const KINDS: [string, string][] = [
  ['', 'Detect automatically'],
  ['linkedin_csv', 'LinkedIn export (.zip or Connections.csv)'],
  ['mbox', 'Mail archive (.mbox, Google Takeout)'],
  ['ics', 'Calendar (.ics)'],
  ['vcf', 'Contacts (.vcf vCard)'],
  ['x_archive', 'X / Twitter archive'],
];

const CATALOG: [string, string, string][] = [
  ['LinkedIn', 'Connections.csv: names, titles, companies, connection dates.', 'linkedin_csv'],
  ['Mail', 'mbox / Google Takeout: headers by default; bodies only if you opt in.', 'mbox'],
  ['Calendar', 'ICS: meeting dates, titles and attendees.', 'ics'],
  ['Contacts', 'vCard from Apple or Google Contacts.', 'vcf'],
  ['X / Twitter', 'Archive: handles, follows and DM metadata.', 'x_archive'],
];

function sourceLine(s: SourceRow): string {
  if (s.kind.startsWith('enrich')) return `${s.kind} · enrichment results · ${shortDate(s.imported_at)}`;
  const st = s.stats;
  return `${s.kind} · imported ${shortDate(s.imported_at)} · ${n(st?.interactions_new)} interactions · ${n(st?.identities_new)} identities`;
}

export function Sources() {
  const { api } = useApp();
  const sources = useLoad(() => api.sources(), [api]);
  return (
    <>
      <PageHead
        title="Bring your network with you."
        sub="Start with exports you already own. Files are streamed to openstance serve on this machine and run through the same importers as the CLI."
      />
      <div className="section-grid">
        <ImportPanel onImported={sources.reload} />
        <section className="panel">
          <div className="panel-icon">
            <Icon name="code" />
          </div>
          <h2>Prefer the terminal?</h2>
          <p>The same importers, from the command line:</p>
          <pre className="query-code">
            openstance import ~/exports/Connections.csv --label "linkedin 2026-10"{'\n'}openstance import ~/exports/*.mbox
          </pre>
          <p className="note">Re-importing a file that was already imported is detected by its hash and skipped.</p>
        </section>
      </div>

      <h3 className="subtle-label">Your imported sources</h3>
      <section className="panel" aria-label="Your imported sources">
        <ErrorNote error={sources.error} />
        {sources.loading && !sources.data && <Loading what="Loading sources" />}
        {sources.data && !sources.data.length && (
          <Empty title="No sources yet." icon="database">
            Import an export above to start your graph.
          </Empty>
        )}
        {sources.data?.map((s) => (
          <div className="source-row" key={s.id}>
            <div>
              {s.label ?? s.path ?? `source #${s.id}`}
              <small>{sourceLine(s)}</small>
            </div>
            <span className="tag">{s.kind.startsWith('enrich') ? 'Enrichment' : 'Local import'}</span>
          </div>
        ))}
      </section>

      <h3 className="subtle-label">Export first · importers</h3>
      <div className="importer-grid">
        {CATALOG.map(([t, d, k]) => (
          <article key={k}>
            <span className="tag">Available</span>
            <h3>{t}</h3>
            <p>{d}</p>
          </article>
        ))}
        <article>
          <span className="tag neutral">Planned</span>
          <h3>WhatsApp, Slack, live mail sync</h3>
          <p>Not in this release. openstance does not scrape networks or sync accounts.</p>
        </article>
      </div>
    </>
  );
}

function ImportPanel({ onImported }: { onImported: () => void }) {
  const { api, toast, refreshStats, config } = useApp();
  const db = dbPath(config);
  const [files, setFiles] = useState<File[]>([]);
  const [kind, setKind] = useState('');
  const [label, setLabel] = useState('');
  const [bodies, setBodies] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [result, setResult] = useState<ImportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function upload() {
    setError(null);
    setResult(null);
    setProgress(0);
    try {
      const r = await api.importFiles(files, { kind: kind || undefined, label: label.trim() || undefined, bodies }, (p) =>
        setProgress(p.total ? Math.round((p.loaded / p.total) * 100) : 0),
      );
      setResult(r);
      setFiles([]);
      toast(`Imported ${r.sources.length} file${r.sources.length === 1 ? '' : 's'} in ${r.took_ms} ms.`);
      onImported();
      refreshStats();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setProgress(null);
    }
  }

  const count = files.length;
  return (
    <section className="panel" aria-label="Import your contacts">
      <div className="panel-icon">
        <Icon name="upload" />
      </div>
      <h2>Import your contacts</h2>
      <p>LinkedIn, mail, calendar, vCard or X archive exports. Subjects and titles are kept; message bodies only if you opt in.</p>
      <form
        className="import-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (count) void upload();
        }}
      >
        <div className="dropzone">
          <label className="field" htmlFor="import-files">
            Choose export files
          </label>
          <input
            id="import-files"
            type="file"
            multiple
            aria-describedby="import-files-hint"
            onChange={(e) => {
              setFiles(Array.from(e.target.files ?? []));
              e.target.value = '';
            }}
          />
          <small className="muted" id="import-files-hint">
            {count > 0
              ? `Selected: ${files.map((f) => f.name).join(', ')}`
              : 'LinkedIn export .zip or Connections.csv, mail .mbox, calendar .ics, contacts .vcf or an X archive.'}
          </small>
        </div>
        <label className="field">
          Format
          <select value={kind} onChange={(e) => setKind(e.target.value)}>
            {KINDS.map(([k, t]) => (
              <option key={k} value={k}>
                {t}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          Label
          <input type="text" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="e.g. linkedin 2026-10" maxLength={200} />
        </label>
        <label className="check">
          <input type="checkbox" checked={bodies} onChange={(e) => setBodies(e.target.checked)} />
          Index message bodies (mail only; off by default)
        </label>
        <div className="modal-actions" style={{ justifyContent: 'flex-start' }}>
          <button type="submit" className="btn primary" disabled={!count || progress !== null}>
            <Icon name="upload" />
            Import {count || ''} {count === 1 ? 'file' : 'files'}
          </button>
        </div>
      </form>
      {progress !== null && (
        <div>
          <progress aria-label="Upload progress" max={100} value={progress} />
          <small className="muted">{progress < 100 ? `Uploading… ${progress}%` : 'Importing and resolving people…'}</small>
        </div>
      )}
      {error && (
        <div className="alert" role="alert">
          {error}
        </div>
      )}
      {result && (
        <section aria-label="Import result">
          <div className="import-stats">
            <div>
              <b>{n(result.resolution.people_created)}</b>
              <span>new people</span>
            </div>
            <div>
              <b>{n(result.resolution.auto_merged)}</b>
              <span>identities merged automatically</span>
            </div>
            <div>
              <a href="#/review" aria-label={`${n(result.resolution.flagged)} flagged for review`}>
                <b>{n(result.resolution.flagged)}</b>
                <span>flagged for review</span>
              </a>
            </div>
          </div>
          {result.sources.map((s) => (
            <div className="source-row" key={s.id}>
              <div>
                {s.path ?? s.label ?? `source #${s.id}`}
                <small>
                  {s.kind} · {s.label ?? 'no label'} · {n(s.identities)} identities · {n(s.interactions)} interactions
                </small>
              </div>
              {s.skipped_duplicate_file ? <span className="tag warn">already imported · skipped</span> : <span className="tag">imported</span>}
            </div>
          ))}
          <p className="note">
            Took {n(result.took_ms)} ms.{' '}
            {db ? (
              <>
                Everything stays in <code>{db}</code>.
              </>
            ) : (
              'Everything stays in your local graph.db on this device.'
            )}
          </p>
        </section>
      )}
    </section>
  );
}
