import { describe, expect, it } from 'vitest';
import { fx } from '../test/fixtures';
import { evidenceLine, initials, shortDate, sourceLabel, usd } from './format';

describe('format', () => {
  it('formats micro-USD as dollars with at least two decimals', () => {
    expect(usd(10000)).toBe('$0.01');
    expect(usd(25000)).toBe('$0.025');
    expect(usd(12345)).toBe('$0.0123');
    expect(usd(0)).toBe('$0.00');
    expect(usd(1_500_000)).toBe('$1.50');
  });

  it('renders interaction evidence like the CLI line', () => {
    const ev = fx.search().results[0]!.evidence[0]!;
    expect(evidenceLine(ev)).toBe('2026-09-14 · meeting mutual · "devtools dinner" · work calendar');
  });

  it('prefers the server-rendered line when present', () => {
    const ev = { ...fx.search().results[0]!.evidence[0]!, line: '2026-09-14  meeting mutual  "devtools dinner"  (work calendar)' };
    expect(evidenceLine(ev)).toBe('2026-09-14  meeting mutual  "devtools dinner"  (work calendar)');
  });

  it('renders tie, field and enrich evidence', () => {
    expect(evidenceLine({ kind: 'field', field: 'company', value: 'Stripe', matched: 'stripe' })).toBe(
      'company: Stripe (matched "stripe")',
    );
    expect(
      evidenceLine({ kind: 'tie', with: { id: 77, display_name: 'Ana Lee' }, tie_kind: 'email', n_shared: 9, weight: 0.52, last_at: '2026-06-02T08:00:00Z' }),
    ).toBe('tie with Ana Lee · 9 shared email · last 2026-06-02');
    expect(
      evidenceLine({ kind: 'enrich', op: 'profile', field: 'company', value: 'Stripe', backend: 'treg', at: '2026-10-19T12:00:00Z', enrich_log_id: 311 }),
    ).toBe('company: Stripe · enrich:treg 2026-10-19');
  });

  it('initials, dates and sources', () => {
    expect(initials('Tom Okafor')).toBe('TO');
    expect(initials(null)).toBe('?');
    expect(shortDate('2026-09-14T16:02:00Z')).toBe('2026-09-14');
    expect(shortDate(null)).toBe('unknown');
    expect(sourceLabel('linkedin_csv:linkedin 2026-10')).toBe('linkedin 2026-10 (linkedin_csv)');
    expect(sourceLabel('override')).toBe('your edit');
    expect(sourceLabel(null)).toBe('unknown');
  });
});
