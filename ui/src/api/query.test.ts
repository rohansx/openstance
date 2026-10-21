import { describe, expect, it } from 'vitest';
import { fx } from '../test/fixtures';
import { addChip, chipsFromQuery, dateInput, editChip, emptyQuery, queryToRequest, removeChip } from './query';

describe('query chips', () => {
  it('turns every set filter of the parsed query into a chip', () => {
    const q = {
      ...emptyQuery(),
      text: 'fintech',
      title: ['founder'],
      industry: ['fintech'],
      since: '2025-01-01T00:00:00Z',
      met_in_person: true,
      min_score: 0.3,
    };
    expect(chipsFromQuery(q).map((c) => c.label)).toEqual([
      'title: founder',
      'industry: fintech',
      'since: 2025-01-01',
      'met in person',
      'min score: 0.3',
    ]);
  });

  it('reads unix-seconds dates (Rust Query) and ISO dates (spec) the same way', () => {
    expect(dateInput(1735689600)).toBe('2025-01-01');
    expect(dateInput('2025-01-01T00:00:00Z')).toBe('2025-01-01');
    expect(dateInput('2025-03')).toBe('2025-03');
    expect(dateInput(null)).toBe('');
  });

  it('builds a no-parse rerun request from the parsed query', () => {
    const q = fx.search().query;
    expect(queryToRequest(q)).toEqual({
      text: 'devtools dinner',
      met_in_person: true,
      include_bulk: false,
      limit: 30,
      parse: false,
    });
  });

  it('removing a chip drops just that filter', () => {
    const q = { ...emptyQuery(), company: ['Stripe', 'Ramp'], met_in_person: true };
    const chips = chipsFromQuery(q);
    const ramp = chips.find((c) => c.label === 'company: Ramp')!;
    const next = removeChip(q, ramp);
    expect(next.company).toEqual(['Stripe']);
    expect(next.met_in_person).toBe(true);
    const met = chipsFromQuery(next).find((c) => c.field === 'met_in_person')!;
    expect(removeChip(next, met).met_in_person).toBeNull();
  });

  it('editing a chip replaces its value; dates go out as strings', () => {
    const q = { ...emptyQuery(), company: ['Stripe'], since: 1735689600 };
    const chips = chipsFromQuery(q);
    let next = editChip(q, chips[0]!, 'Plaid');
    next = editChip(next, chipsFromQuery(next).find((c) => c.field === 'since')!, '2024-06-01');
    expect(queryToRequest(next)).toMatchObject({ company: ['Plaid'], since: '2024-06-01', parse: false });
  });

  it('adds a chip, ignoring blanks and duplicates', () => {
    let q = addChip(emptyQuery(), 'title', ' founder ');
    q = addChip(q, 'title', 'founder');
    q = addChip(q, 'title', '   ');
    expect(q.title).toEqual(['founder']);
    expect(addChip(emptyQuery(), 'replied', 'true').replied).toBe(true);
    expect(addChip(emptyQuery(), 'min_score', '0.5').min_score).toBe(0.5);
  });
});
