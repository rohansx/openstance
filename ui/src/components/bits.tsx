import type { ReactNode } from 'react';
import type { Evidence, Person } from '../api/types';
import { evidenceLine, initials, pct } from '../lib/format';
import { Icon, type IconName } from './Icon';

export function Avatar({ name, tone = 0, className = '' }: { name: string | null | undefined; tone?: number; className?: string }) {
  return (
    <div className={`avatar tone-${tone % 4} ${className}`.trim()} aria-hidden="true">
      {initials(name)}
    </div>
  );
}

/** Relationship score as a bar (0..1 -> /100). */
export function ScoreBar({ score, label = 'Relationship score' }: { score: number; label?: string }) {
  const v = pct(score);
  return (
    <span className="score-bar" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={v}>
      <span className="score-bar-track">
        <i style={{ width: `${v}%` }} />
      </span>
      <b>{v}</b>
      <span className="score-bar-max">/100</span>
    </span>
  );
}

export function EvidenceList({ items, empty }: { items: Evidence[]; empty?: string }) {
  if (!items.length) return empty ? <p className="muted evidence-empty">{empty}</p> : null;
  return (
    <ul className="evidence-lines">
      {items.map((ev, i) => (
        <li key={i} className={`ev ev-${ev.kind}`}>
          <Icon name={ev.kind === 'tie' ? 'route' : ev.kind === 'enrich' ? 'external' : 'check'} />
          <span>{evidenceLine(ev)}</span>
        </li>
      ))}
    </ul>
  );
}

export function PersonFlags({ person }: { person: Pick<Person, 'do_not_share' | 'no_enrich' | 'is_bulk'> }) {
  return (
    <>
      {person.do_not_share && <span className="tag warn">Do not share · excluded from views</span>}
      {person.no_enrich && <span className="tag neutral">No enrichment</span>}
      {person.is_bulk && <span className="tag neutral">Bulk sender</span>}
    </>
  );
}

export function Empty({
  title,
  children,
  icon = 'people',
  action,
}: {
  title: string;
  children?: ReactNode;
  icon?: IconName;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <Icon name={icon} />
      <h3>{title}</h3>
      {children && <p>{children}</p>}
      {action}
    </div>
  );
}

export function PageHead({
  eyebrow,
  title,
  sub,
  aside,
}: {
  eyebrow?: string;
  title: string;
  sub?: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <div className="page-head">
      <div>
        {eyebrow && <div className="eyebrow">{eyebrow}</div>}
        <h1 className={eyebrow ? 'with-eyebrow' : undefined}>{title}</h1>
        {sub && <p>{sub}</p>}
      </div>
      {aside}
    </div>
  );
}

export function ErrorNote({ error }: { error: Error | undefined }) {
  if (!error) return null;
  return (
    <div className="alert" role="alert">
      {error.message}
    </div>
  );
}

export function Loading({ what = 'Loading' }: { what?: string }) {
  return (
    <p className="loading muted" role="status">
      {what}…
    </p>
  );
}
