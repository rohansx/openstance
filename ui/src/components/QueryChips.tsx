import { useState } from 'react';
import { addChip, chipsFromQuery, editChip, FIELD_LABEL, removeChip, type Chip, type ChipField } from '../api/query';
import type { Query } from '../api/types';
import { Icon } from './Icon';

const ADDABLE: ChipField[] = [
  'company',
  'title',
  'location',
  'industry',
  'tags',
  'since',
  'until',
  'last_contact_before',
  'met_in_person',
  'replied',
  'lost_touch',
  'min_score',
  'include_bulk',
];

const PLACEHOLDER: Partial<Record<ChipField, string>> = {
  since: '2025, 2025-06, 90d',
  until: '2026-01-01',
  last_contact_before: '2025-01-01 or 6m',
  met_in_person: 'true / false',
  replied: 'true / false',
  lost_touch: 'true',
  include_bulk: 'true',
  min_score: '0..1',
};

function ChipButton({ chip, onEdit, onRemove }: { chip: Chip; onEdit: (v: string) => void; onRemove: () => void }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(chip.value);
  if (editing) {
    return (
      <span className="chip-edit">
        <input
          aria-label={`${FIELD_LABEL[chip.field]} value`}
          value={value}
          autoFocus
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              setEditing(false);
              if (value !== chip.value) onEdit(value);
            } else if (e.key === 'Escape') {
              setEditing(false);
              setValue(chip.value);
            }
          }}
          onBlur={() => {
            setEditing(false);
            if (value !== chip.value) onEdit(value);
          }}
        />
      </span>
    );
  }
  return (
    <span className="chip-edit">
      <button type="button" aria-label={`Edit filter ${chip.label}`} title="Edit filter" onClick={() => setEditing(true)}>
        {chip.label}
      </button>
      <button type="button" aria-label={`Remove filter ${chip.label}`} title="Remove filter" onClick={onRemove}>
        ×
      </button>
    </span>
  );
}

/** The parsed `Query` as editable chips. Every change reruns with `parse: false`. */
export function QueryChips({
  query,
  parser,
  fallback,
  onChange,
}: {
  query: Query;
  parser: string;
  fallback: string | null;
  onChange: (q: Query) => void;
}) {
  const chips = chipsFromQuery(query);
  const [field, setField] = useState<ChipField>('company');
  const [value, setValue] = useState('');

  return (
    <>
      <div className="query-interpretation" role="group" aria-label="Parsed filters">
        <span>PARSED FILTERS</span>
        {chips.length ? (
          chips.map((c) => (
            <ChipButton
              key={`${c.field}:${c.value}`}
              chip={c}
              onEdit={(v) => onChange(editChip(query, c, v))}
              onRemove={() => onChange(removeChip(query, c))}
            />
          ))
        ) : (
          <small>No filters · text only</small>
        )}
        {query.text && <small>text: “{query.text}”</small>}
        <small>
          parser: {parser}
          {fallback ? ` · fell back: ${fallback}` : ''} · edits rerun without a model
        </small>
      </div>
      <form
        className="add-filter"
        onSubmit={(e) => {
          e.preventDefault();
          const next = addChip(query, field, value);
          setValue('');
          if (next !== query) onChange(next);
        }}
      >
        <select aria-label="Filter field" value={field} onChange={(e) => setField(e.target.value as ChipField)}>
          {ADDABLE.map((f) => (
            <option key={f} value={f}>
              {FIELD_LABEL[f]}
            </option>
          ))}
        </select>
        <input
          type="text"
          aria-label="Filter value"
          value={value}
          placeholder={PLACEHOLDER[field] ?? 'value'}
          onChange={(e) => setValue(e.target.value)}
        />
        <button type="submit" className="btn">
          <Icon name="plus" />
          Add filter
        </button>
      </form>
    </>
  );
}
