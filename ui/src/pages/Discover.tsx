import { useEffect, useRef, useState } from 'react';
import { emptyQuery, queryToRequest } from '../api/query';
import type { Query, SearchRequest, SearchResponse } from '../api/types';
import { useApp } from '../app/context';
import { Empty, ErrorNote, Loading, PageHead } from '../components/bits';
import { Icon } from '../components/Icon';
import { QueryChips } from '../components/QueryChips';
import { ResultCard } from '../components/ResultCard';
import { SaveViewDialog } from '../components/SaveViewDialog';
import { initials } from '../lib/format';
import { useShortlist } from '../lib/useShortlist';

const EXAMPLES: [string, 'spark' | 'leaf' | 'people' | 'clock'][] = [
  ["fintech founders I've met", 'spark'],
  ['devtools investors', 'leaf'],
  ['engineering managers who replied', 'people'],
  ['people I lost touch with', 'clock'],
];

export function Discover({ mode }: { mode: 'discover' | 'people' }) {
  const { api, stats, refreshStats } = useApp();
  const shortlist = useShortlist();
  const [text, setText] = useState('');
  const [res, setRes] = useState<SearchResponse | undefined>();
  const [error, setError] = useState<Error | undefined>();
  const [busy, setBusy] = useState(false);
  const [saveOpen, setSaveOpen] = useState(false);
  const seq = useRef(0);

  async function run(req: SearchRequest) {
    const mine = ++seq.current;
    setBusy(true);
    setError(undefined);
    try {
      const r = await api.search(req);
      if (mine === seq.current) setRes(r);
    } catch (e) {
      if (mine === seq.current) setError(e as Error);
    } finally {
      if (mine === seq.current) setBusy(false);
    }
  }

  const runRef = useRef(run);
  useEffect(() => {
    runRef.current = run;
  });

  // Start with a structured search: everyone, strongest relationships first.
  useEffect(() => {
    void runRef.current(queryToRequest({ ...emptyQuery(), limit: mode === 'people' ? 50 : 10 }));
  }, [mode]);

  const submitText = (t: string) => {
    setText(t);
    if (!t.trim()) {
      void run(queryToRequest({ ...emptyQuery(), limit: 30 }));
      return;
    }
    void run({ text: t.trim(), parse: true, limit: 30 });
  };

  const rerun = (q: Query) => void run(queryToRequest(q, false));

  const results = res?.results ?? [];
  const isText = res?.mode === 'text';

  return (
    <>
      {stats && stats.people === 0 && (
        <div className="mode-banner">
          <span>
            <strong>Your graph is empty.</strong> Import a LinkedIn, mail, calendar or vCard export to start.
          </span>
          <a className="btn text" href="#/sources">
            Import contacts <Icon name="arrow" />
          </a>
        </div>
      )}
      {mode === 'discover' ? (
        <section className="hero work-hero">
          <div className="eyebrow">YOUR PEOPLE. WITH THE CONTEXT.</div>
          <h1>Who are you looking for?</h1>
          <p>Search your graph. Refine the filters. Follow the evidence.</p>
          <div className="engine-badge">
            <span className="dot" />
            Local search <span>·</span> {res?.models.parser ?? 'rules'} parser <span>·</span>{' '}
            {res?.models.embedder ?? 'embedder'}
          </div>
        </section>
      ) : (
        <PageHead title="Your people, in one place." sub="Everyone in your graph, strongest relationship first. Search, correct, and reconnect." />
      )}

      <form
        className="search-box"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          submitText(text);
        }}
      >
        <div className="search-top">
          <Icon name="search" />
          <input
            type="search"
            aria-label="Search your network"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Try a role, company, place, or what you talked about…"
            autoComplete="off"
          />
          <button type="submit" className="btn primary" aria-label="Find people">
            Find people <Icon name="arrow" />
          </button>
        </div>
        <div className="search-foot">
          <span>
            <Icon name="database" /> {stats ? `${stats.people.toLocaleString('en-US')} people` : 'Your graph'} · full-text
            {res && res.models.embedder !== 'none' ? ' + semantic' : ''} search
          </span>
          <span>
            <Icon name="shield" /> No search credits. Search runs on this device.
          </span>
        </div>
      </form>

      {mode === 'discover' && (
        <div className="examples">
          <span>Try a search</span>
          {EXAMPLES.map(([q, icon]) => (
            <button type="button" className="chip" key={q} onClick={() => submitText(q)}>
              <Icon name={icon} />
              {q}
            </button>
          ))}
        </div>
      )}

      <div className="results-layout">
        <section aria-label="Search results">
          <div className="result-header">
            <h2>
              {isText ? 'Your next connection' : mode === 'people' ? 'Everyone' : 'Your strongest relationships'}{' '}
              <small>
                {res ? `${res.total} ${res.total === 1 ? 'person' : 'people'} · ${res.took_ms} ms` : ''}
              </small>
            </h2>
            <div className="result-actions">
              <button type="button" className="btn" onClick={() => setSaveOpen(true)} disabled={!res}>
                <Icon name="bookmark" />
                Save view
              </button>
            </div>
          </div>

          {res && (
            <>
              <QueryChips
                query={res.query}
                parser={res.query_source.parser}
                fallback={res.query_source.fallback}
                onChange={rerun}
              />
              {res.rerun && <p className="rerun-line">$ {res.rerun}</p>}
            </>
          )}

          {res && (res.warnings.length > 0 || res.suggestions.length > 0) && (
            <div className="warnings">
              {res.warnings.map((w) => (
                <div className="alert" key={w}>
                  {w}
                </div>
              ))}
              {res.suggestions.map((s) => (
                <div className="alert info" key={s}>
                  {s}
                </div>
              ))}
            </div>
          )}

          <ErrorNote error={error} />
          <div aria-live="polite" aria-busy={busy}>
            {busy && !res && <Loading what="Searching" />}
            {res &&
              (results.length ? (
                results.map((r, i) => (
                  <ResultCard
                    key={r.person.id}
                    result={r}
                    index={i}
                    saved={shortlist.ids.has(r.person.id)}
                    onToggleSave={() => void shortlist.toggle(r.person)}
                  />
                ))
              ) : (
                <Empty title="No matches in your graph.">
                  Remove a filter chip, or try fewer words. Every chip edit reruns locally without a model.
                </Empty>
              ))}
          </div>
        </section>
        <Insights results={res} />
      </div>
      <div className="workspace-footer">
        <Icon name="leaf" /> Built for relationships, not contact collecting.
      </div>
      {saveOpen && res && (
        <SaveViewDialog
          query={res.query}
          onClose={() => setSaveOpen(false)}
          onSaved={() => {
            setSaveOpen(false);
            refreshStats();
          }}
        />
      )}
    </>
  );
}

function Insights({ results }: { results: SearchResponse | undefined }) {
  const { stats } = useApp();
  const top = (results?.results ?? []).slice(0, 5);
  const coords: [number, number][] = [
    [45, 40],
    [205, 40],
    [29, 125],
    [215, 125],
    [125, 155],
  ];
  return (
    <aside className="insights">
      <section className="support-card">
        <div className="support-title">
          Around you <Icon name="graph" />
        </div>
        <svg
          className="network-graph"
          viewBox="0 0 250 180"
          role="img"
          aria-label="The top results around you. The layout is illustrative; open a person for evidence-backed paths."
        >
          {top.map((r, i) => {
            const [x, y] = coords[i]!;
            return (
              <g key={r.person.id}>
                <line x1="125" y1="83" x2={x} y2={y} />
                <circle cx={x} cy={y} r="19" />
                <text x={x} y={y + 4} textAnchor="middle">
                  {initials(r.person.display_name)}
                </text>
              </g>
            );
          })}
          <circle className="center" cx="125" cy="83" r="25" />
          <text className="you" x="125" y="87" textAnchor="middle">
            You
          </text>
        </svg>
        <div className="stats">
          <div>
            <strong>{stats ? stats.people.toLocaleString('en-US') : '–'}</strong>
            <span>people in your graph</span>
          </div>
          <div>
            <strong>{stats ? stats.sources : '–'}</strong>
            <span>imported sources</span>
          </div>
        </div>
        <p className="note">The layout is illustrative. A path is a lead, not a promise of an intro.</p>
      </section>
      <section className="support-card white">
        <h3>A little more signal. A lot less noise.</h3>
        {(
          [
            ['check', 'Know why they matched', 'Every result carries dated evidence from your own exports.'],
            ['route', 'Make the next move', 'Shortlist a person, check how you are connected, track the intro.'],
            ['lock', 'Keep your network yours', 'One file on this device. Nothing leaves without a quote.'],
          ] as const
        ).map(([i, t, d]) => (
          <div className="benefit" key={t}>
            <Icon name={i} />
            <div>
              <strong>{t}</strong>
              <small>{d}</small>
            </div>
          </div>
        ))}
        <a className="btn" href="#/sources">
          <Icon name="plus" />
          Explore your data sources
        </a>
      </section>
    </aside>
  );
}
