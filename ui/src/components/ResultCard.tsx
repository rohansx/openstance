import { href } from '../app/router';
import type { SearchResult } from '../api/types';
import { displayName, shortDate, sourceLabel } from '../lib/format';
import { Avatar, EvidenceList, PersonFlags, ScoreBar } from './bits';
import { Icon } from './Icon';

export function ResultCard({
  result,
  index,
  saved,
  onToggleSave,
}: {
  result: SearchResult;
  index: number;
  saved: boolean;
  onToggleSave: () => void;
}) {
  const p = result.person;
  const name = displayName(p);
  const link = href({ name: 'person', id: String(p.id) });
  const role = [p.title, p.company].filter(Boolean).join(' · ');
  const companySource = p.field_sources?.company ?? p.field_sources?.title ?? null;
  return (
    <article className="person-card">
      <div className="person-head">
        <Avatar name={name} tone={index} />
        <div>
          <h3 className="person-name">
            <a href={link}>{name}</a>
          </h3>
          <div className="person-role">{role || 'Title and company unknown'}</div>
          <div className="person-location">
            <Icon name="pin" />
            {p.location || 'Location unknown'}
          </div>
        </div>
        <button
          type="button"
          className={`save${saved ? ' saved' : ''}`}
          aria-pressed={saved}
          aria-label={saved ? `Remove ${name} from shortlist` : `Save ${name} to shortlist`}
          onClick={onToggleSave}
        >
          <Icon name="bookmark" />
        </button>
      </div>
      <div className="record-signals">
        <ScoreBar score={p.score} />
        {result.relevance && (
          <span className="relevance" title="fused relevance × (0.5 + 0.5 × score)">
            relevance {result.relevance.final.toFixed(3)}
          </span>
        )}
        <PersonFlags person={p} />
      </div>
      {result.matched.length > 0 && (
        <div className="evidence">
          <Icon name="check" />
          <span>Matched</span>
          {result.matched.map((m, i) => (
            <span className="tag" key={i}>
              {m.field}
              {m.matched ? `: ${m.matched}` : m.via ? `: ${m.via}` : ''}
            </span>
          ))}
        </div>
      )}
      <EvidenceList items={result.evidence} empty="No interactions in evidence for this match." />
      {result.reason && (
        <p className="reason">
          {result.reason} <small>({result.reason_source ?? 'model'})</small>
        </p>
      )}
      <div className="record-source">
        <Icon name="database" /> {sourceLabel(companySource)} <span>·</span> Last contact {shortDate(p.last_contact)}
      </div>
      <div className="card-bottom">
        <span className="path-label">
          {p.tags.slice(0, 4).map((t) => (
            <span className="tag" key={t}>
              {t}
            </span>
          ))}
        </span>
        <a className="btn" href={link} aria-label={`Open ${name}`}>
          Open person <Icon name="arrow" />
        </a>
      </div>
    </article>
  );
}
