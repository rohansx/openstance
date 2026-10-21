// The parsed `Query` as editable chips (P4-UI-02): the user edits filters and
// reruns with `parse: false`, so no model call is needed for a rerun.

import type { DateValue, Query, SearchRequest } from './types';

export const ARRAY_FIELDS = ['company', 'title', 'location', 'industry', 'tags'] as const;
export const DATE_FIELDS = ['since', 'until', 'last_contact_before'] as const;
export const BOOL_FIELDS = ['met_in_person', 'replied', 'lost_touch', 'include_bulk'] as const;

export type ArrayField = (typeof ARRAY_FIELDS)[number];
export type DateField = (typeof DATE_FIELDS)[number];
export type BoolField = (typeof BOOL_FIELDS)[number];
export type ChipField = ArrayField | DateField | BoolField | 'min_score';

export interface Chip {
  field: ChipField;
  /** The value as the user edits it (string form). */
  value: string;
  label: string;
}

export const FIELD_LABEL: Record<ChipField, string> = {
  company: 'company',
  title: 'title',
  location: 'location',
  industry: 'industry',
  tags: 'tag',
  since: 'since',
  until: 'until',
  last_contact_before: 'not since',
  met_in_person: 'met in person',
  replied: 'replied',
  lost_touch: 'lost touch',
  include_bulk: 'include bulk',
  min_score: 'min score',
};

export function emptyQuery(): Query {
  return {
    text: null,
    company: [],
    title: [],
    location: [],
    industry: [],
    tags: [],
    since: null,
    until: null,
    last_contact_before: null,
    met_in_person: null,
    replied: null,
    min_score: null,
    lost_touch: null,
    include_bulk: false,
    limit: 30,
  };
}

/** Normalize a date filter for display and for the request: YYYY[-MM[-DD]] or a relative `90d`. */
export function dateInput(v: DateValue | undefined): string {
  if (v === null || v === undefined || v === '') return '';
  if (typeof v === 'number') return new Date(v * 1000).toISOString().slice(0, 10);
  const iso = /^(\d{4}-\d{2}-\d{2})T/.exec(v);
  return iso ? iso[1]! : v;
}

function boolLabel(field: BoolField, v: boolean): string {
  if (field === 'met_in_person') return v ? 'met in person' : 'never met';
  if (field === 'replied') return v ? 'replied' : 'never replied';
  if (field === 'lost_touch') return v ? 'lost touch' : 'in touch';
  return v ? 'include bulk' : 'no bulk';
}

export function chipsFromQuery(q: Query): Chip[] {
  const chips: Chip[] = [];
  for (const f of ARRAY_FIELDS) {
    for (const v of q[f] ?? []) chips.push({ field: f, value: v, label: `${FIELD_LABEL[f]}: ${v}` });
  }
  for (const f of DATE_FIELDS) {
    const v = dateInput(q[f]);
    if (v) chips.push({ field: f, value: v, label: `${FIELD_LABEL[f]}: ${v}` });
  }
  for (const f of BOOL_FIELDS) {
    const v = q[f];
    if (f === 'include_bulk' ? v === true : typeof v === 'boolean') {
      chips.push({ field: f, value: String(v), label: boolLabel(f, v as boolean) });
    }
  }
  if (typeof q.min_score === 'number') {
    chips.push({ field: 'min_score', value: String(q.min_score), label: `min score: ${q.min_score}` });
  }
  return chips;
}

const isArrayField = (f: ChipField): f is ArrayField => (ARRAY_FIELDS as readonly string[]).includes(f);
const isDateField = (f: ChipField): f is DateField => (DATE_FIELDS as readonly string[]).includes(f);

export function removeChip(q: Query, chip: Chip): Query {
  const next: Query = { ...q };
  if (isArrayField(chip.field)) {
    const f = chip.field;
    next[f] = q[f].filter((v) => v !== chip.value);
  } else if (isDateField(chip.field)) {
    next[chip.field] = null;
  } else if (chip.field === 'min_score') {
    next.min_score = null;
  } else if (chip.field === 'include_bulk') {
    next.include_bulk = false;
  } else {
    next[chip.field] = null;
  }
  return next;
}

function parseBool(v: string): boolean | null {
  const s = v.trim().toLowerCase();
  if (['true', 'yes', 'y', '1'].includes(s)) return true;
  if (['false', 'no', 'n', '0'].includes(s)) return false;
  return null;
}

export function addChip(q: Query, field: ChipField, rawValue: string): Query {
  const value = rawValue.trim();
  if (!value) return q;
  const next: Query = { ...q };
  if (isArrayField(field)) {
    if (!q[field].some((v) => v.toLowerCase() === value.toLowerCase())) next[field] = [...q[field], value];
  } else if (isDateField(field)) {
    next[field] = value;
  } else if (field === 'min_score') {
    const n = Number(value);
    if (Number.isFinite(n) && n >= 0 && n <= 1) next.min_score = n;
  } else if (field === 'include_bulk') {
    next.include_bulk = parseBool(value) ?? false;
  } else {
    next[field] = parseBool(value);
  }
  return next;
}

export function editChip(q: Query, chip: Chip, value: string): Query {
  if (!value.trim()) return removeChip(q, chip);
  if (isArrayField(chip.field)) {
    const f = chip.field;
    const v = value.trim();
    return { ...q, [f]: q[f].map((x) => (x === chip.value ? v : x)) };
  }
  return addChip(removeChip(q, chip), chip.field, value);
}

/** The request that reruns this query literally (no parser, no model). */
export function queryToRequest(q: Query, parse = false): SearchRequest {
  const req: SearchRequest = {};
  if (q.text && q.text.trim()) req.text = q.text.trim();
  for (const f of ARRAY_FIELDS) if (q[f]?.length) req[f] = [...q[f]];
  for (const f of DATE_FIELDS) {
    const v = dateInput(q[f]);
    if (v) req[f] = v;
  }
  if (typeof q.met_in_person === 'boolean') req.met_in_person = q.met_in_person;
  if (typeof q.replied === 'boolean') req.replied = q.replied;
  if (typeof q.min_score === 'number') req.min_score = q.min_score;
  if (typeof q.lost_touch === 'boolean') req.lost_touch = q.lost_touch;
  req.include_bulk = q.include_bulk === true;
  req.limit = q.limit || 30;
  req.parse = parse;
  return req;
}
