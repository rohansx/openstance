import { configValue, useApp } from '../app/context';
import { PageHead } from '../components/bits';
import { Icon } from '../components/Icon';
import { shortDate, usd } from '../lib/format';

function flatten(obj: unknown, prefix = ''): [string, string][] {
  if (obj === null || typeof obj !== 'object' || Array.isArray(obj)) {
    return [[prefix, Array.isArray(obj) ? JSON.stringify(obj) : String(obj)]];
  }
  const o = obj as Record<string, unknown>;
  if ('value' in o && Object.keys(o).length <= 2) return [[prefix, JSON.stringify(o.value)]];
  return Object.entries(o).flatMap(([k, v]) => flatten(v, prefix ? `${prefix}.${k}` : k));
}

export function Settings() {
  const { config, health, stats, setToken } = useApp();
  const cfg = config?.config ?? (config as Record<string, unknown> | undefined);
  const rows = cfg ? flatten(cfg).filter(([k]) => k !== 'v') : [];
  const origins = (config?.origins ?? {}) as Record<string, string>;
  const parser = String(configValue(config, 'model.parser') ?? 'rules');
  const backend = String(configValue(config, 'enrich.backend') ?? 'none');

  return (
    <>
      <PageHead title="Your network. Your terms." sub="One file on this device that you can inspect, back up, export, and serve yourself." />
      <div className="section-grid">
        <section className="panel">
          <h2>On this device</h2>
          <p>
            Your graph lives in <code>~/.openstance/graph.db</code>. This page talks to <code>openstance serve</code> on
            loopback with a bearer token kept in this tab’s session storage.
          </p>
          <div className="source-row">
            <span>openstance</span>
            <b>{health ? `v${health.version} · schema ${health.schema_version}` : 'unknown'}</b>
          </div>
          <div className="source-row">
            <span>People</span>
            <b>{stats?.people.toLocaleString('en-US') ?? '–'}</b>
          </div>
          <div className="source-row">
            <span>Last import</span>
            <b>{stats?.last_import ? shortDate(stats.last_import) : 'never'}</b>
          </div>
          <div className="source-row">
            <span>Spent on enrichment</span>
            <b>{stats ? usd(stats.enrich.spent_micro_usd) : '–'}</b>
          </div>
          <div className="source-row">
            <span>Query parser</span>
            <span className="tag">{parser}</span>
          </div>
          <div className="source-row">
            <span>Enrichment</span>
            <span className={`tag ${backend === 'none' ? 'neutral' : ''}`}>{backend === 'none' ? 'Off' : backend}</span>
          </div>
          <button type="button" className="btn danger" onClick={() => setToken(null)}>
            <Icon name="lock" />
            Forget the token in this tab
          </button>
          <p className="note">
            Run <code>openstance serve --rotate-token</code> to invalidate every open tab and agent using the old token.
          </p>
        </section>
        <section className="panel">
          <h2>Always a way out.</h2>
          <p>Export everything, back it up, or take it elsewhere. These run from the terminal:</p>
          <pre className="query-code">
            openstance export --format csv -o people.csv{'\n'}openstance backup ~/openstance-backup.db{'\n'}openstance config edit
          </pre>
          <p className="note">Exports exclude people marked “Do not share” unless you pass --include-do-not-share.</p>
        </section>
      </div>
      <section className="panel config-list" style={{ marginTop: 22 }} aria-label="Effective config">
        <h2>Effective config</h2>
        <p>
          {config?.config_file ? (
            <>
              From <code>{config.config_file}</code>
              {config.file_found === false ? ' (not found; defaults in use)' : ''}. Secrets are never included; only the names of
              the environment variables that hold them.
            </>
          ) : (
            'Loading…'
          )}
        </p>
        {(config?.warnings ?? []).map((w) => (
          <div className="alert" key={w}>
            {w}
          </div>
        ))}
        {rows.map(([k, v]) => (
          <div className="source-row" key={k}>
            <span>{k}</span>
            <span>
              {v}
              {origins[k] ? <small> · {origins[k]}</small> : null}
            </span>
          </div>
        ))}
      </section>
    </>
  );
}
