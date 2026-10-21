import type { ReactNode } from 'react';
import { configValue, useApp } from '../app/context';
import { href, TITLES, type Route } from '../app/router';
import { Icon, Logo, type IconName } from './Icon';

type NavKey = Exclude<Route['name'], 'person'>;

const NAV: { key: NavKey; label: string; icon: IconName; sepBefore?: boolean }[] = [
  { key: 'discover', label: 'Search people', icon: 'search' },
  { key: 'people', label: 'People', icon: 'people' },
  { key: 'views', label: 'Saved views', icon: 'bookmark' },
  { key: 'playbooks', label: 'Playbooks', icon: 'route' },
  { key: 'shortlist', label: 'Shortlist', icon: 'bookmark' },
  { key: 'intros', label: 'Introductions', icon: 'mail' },
  { key: 'sources', label: 'Sources', icon: 'database', sepBefore: true },
  { key: 'review', label: 'Review queue', icon: 'review' },
  { key: 'integrations', label: 'Integrations', icon: 'graph' },
  { key: 'agents', label: 'Agent access', icon: 'code' },
  { key: 'egress', label: 'Egress log', icon: 'receipt' },
  { key: 'settings', label: 'Settings', icon: 'settings' },
];

const fmt = (n: number) => n.toLocaleString('en-US');

export function Shell({ route, children }: { route: Route; children: ReactNode }) {
  const { stats, health, config } = useApp();
  const current: NavKey = route.name === 'person' ? 'people' : route.name;
  const backend = String(configValue(config, 'enrich.backend') ?? 'none');
  const parser = String(configValue(config, 'model.parser') ?? 'rules');

  const count = (key: NavKey): number | undefined => {
    if (!stats) return undefined;
    if (key === 'people') return stats.people;
    if (key === 'review') return stats.review?.flagged;
    if (key === 'shortlist') return stats.shortlist;
    return undefined;
  };

  return (
    <div className="shell">
      <aside className="sidebar">
        <a className="brand" href="#/discover">
          <Logo />
          openstance
          <small>YOUR NETWORK AS A FILE</small>
        </a>
        <div className="workspace">
          <div className="avatar">ME</div>
          <div>
            <strong>My graph</strong>
            <small>Personal · stored on this device</small>
          </div>
        </div>
        <div className="nav-label">Workspace</div>
        <nav className="nav" aria-label="Main navigation">
          {NAV.map((item) => {
            const n = count(item.key);
            const active = item.key === current;
            return (
              <span key={item.key} className="nav-item">
                {item.sepBefore && <span className="separator" aria-hidden="true" />}
                <a
                  href={href({ name: item.key } as Route)}
                  className={active ? 'active' : undefined}
                  aria-current={active ? 'page' : undefined}
                >
                  <Icon name={item.icon} />
                  {item.label}
                  {n !== undefined && <span className="count">{fmt(n)}</span>}
                </a>
              </span>
            );
          })}
        </nav>
        <div className="sidebar-bottom">
          <div className="local-box">
            <strong>
              <span className="dot" />
              Local mode
            </strong>
            Enrichment {backend === 'none' ? 'off' : backend} · parser {parser}
            <br />
            openstance {health ? `v${health.version}` : 'serve'} · loopback only
          </div>
          <span className="source-link">
            <span>
              <Icon name="code" /> Open source · MIT
            </span>
          </span>
        </div>
      </aside>
      <div className="main-shell">
        <header className="topbar">
          <div className="breadcrumb" aria-label="Breadcrumb">
            Workspace <span>/</span> <b>{TITLES[route.name]}</b>
          </div>
          <div className="top-actions">
            <span className="privacy-pill">
              <Icon name="lock" /> Stored on this device
            </span>
            <a className="btn" href="#/sources">
              <Icon name="plus" />
              Import contacts
            </a>
            <span className="profile" aria-hidden="true">
              ME
            </span>
          </div>
        </header>
        <main className="main" id="main">
          {children}
        </main>
      </div>
    </div>
  );
}
