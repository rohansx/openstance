import type { DateValue, Evidence, MicroUsd } from '../api/types';
import { dateInput } from '../api/query';

/** Integer micro-USD to `$0.01`, `$0.025`, `$0.0123` (2 to 4 decimals). */
export function usd(micro: MicroUsd | null | undefined): string {
  const n = (micro ?? 0) / 1_000_000;
  let s = n.toFixed(4);
  s = s.replace(/0{1,2}$/, '');
  return `$${s}`;
}

export function shortDate(v: DateValue | undefined): string {
  return dateInput(v ?? null) || 'unknown';
}

export function initials(name: string | null | undefined): string {
  const parts = String(name ?? '')
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2);
  return parts.length ? parts.map((w) => w[0]!.toUpperCase()).join('') : '?';
}

/** `override` / `<kind>:<label>` / null, as shown to a person. */
export function sourceLabel(src: string | null | undefined): string {
  if (!src) return 'unknown';
  if (src === 'override') return 'your edit';
  const i = src.indexOf(':');
  if (i < 0) return src;
  return `${src.slice(i + 1)} (${src.slice(0, i)})`;
}

export function evidenceLine(ev: Evidence): string {
  switch (ev.kind) {
    case 'interaction': {
      if (ev.line) return ev.line;
      const parts = [shortDate(ev.at), [ev.type, ev.direction].filter(Boolean).join(' ')];
      if (ev.subject) parts.push(`"${ev.subject}"`);
      if (ev.source?.label) parts.push(ev.source.label);
      return parts.join(' · ');
    }
    case 'field':
      return `${ev.field}: ${ev.value} (matched "${ev.matched}")`;
    case 'tie':
      return `tie with ${ev.with.display_name ?? `#${ev.with.id}`} · ${ev.n_shared} shared ${ev.tie_kind} · last ${shortDate(ev.last_at)}`;
    case 'enrich':
      return `${ev.field}: ${ev.value} · enrich:${ev.backend} ${shortDate(ev.at)}`;
    default:
      return '';
  }
}

export function pct(x: number | null | undefined): number {
  return Math.max(0, Math.min(100, Math.round((x ?? 0) * 100)));
}

export function displayName(p: { display_name: string | null; id: number } | null | undefined): string {
  if (!p) return 'Unknown';
  return p.display_name || `Person #${p.id}`;
}
