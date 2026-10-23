import { useState } from 'react';
import { configValue, useApp } from '../app/context';
import { PageHead } from '../components/bits';
import { Dialog } from '../components/Dialog';
import { Icon } from '../components/Icon';

type Category = 'Introductions' | 'Enrichment' | 'Discovery' | 'Sync';

interface Provider {
  id: string;
  name: string;
  mark: string;
  tone: string;
  category: Category;
  method: string;
  summary: string;
  input: string;
  output: string;
  availability: string;
  url: string;
  /** Static roadmap status; treg's comes from the config. */
  status?: string;
}

// Ported from the v0.3 prototype's provider directory, with statuses that
// match what this build can actually do. Nothing here is a partnership.
const PROVIDERS: Provider[] = [
  {
    id: 'treg',
    name: 'treg',
    mark: 't',
    tone: 'treg',
    category: 'Enrichment',
    method: 'Enrichment router',
    summary: 'Fill gaps in selected records and verify work emails, with a quote and a budget before anything is sent.',
    input:
      'Only the fields a playbook quotes: a name, company, LinkedIn URL, domain or email. Notes, subjects and the rest of your graph stay on this device. People marked “Do not enrich” are never sent.',
    output: 'Profile fields, company context, email find and verification, recorded with provenance (enrich:treg) and in the egress log.',
    availability:
      'Off by default (enrich.backend = "none"). To enable it, set enrich.backend = "treg" and put your token in OPENSTANCE_TREG_TOKEN. Every playbook returns a quote first and sends nothing until you press Spend. Some unsuccessful lookups can still cost money.',
    url: 'https://treg.to',
  },
  {
    id: 'boardy',
    name: 'Boardy',
    mark: 'B',
    tone: 'boardy',
    category: 'Introductions',
    method: 'Email handoff',
    summary: 'Find a willing introduction beyond the people you already know.',
    input: 'A brief you write and review: who you are, who you want to meet, why now, and what you offer.',
    output: 'A reply in your inbox; introductions depend on mutual interest.',
    availability: 'Not built. No public API or webhook documentation was found in our review.',
    url: 'https://www.boardy.ai/agents',
    status: 'Planned',
  },
  {
    id: 'exa',
    name: 'Exa',
    mark: 'e',
    tone: 'exa',
    category: 'Discovery',
    method: 'Search API',
    summary: 'Discover public profiles for a specific role, expertise, or problem.',
    input: 'A search description you review. Contact records are not needed to create the query.',
    output: 'Candidate profiles and source URLs to compare with your local network.',
    availability: 'Not built. Public discovery is not evidence of a warm relationship.',
    url: 'https://exa.ai',
    status: 'Planned',
  },
  {
    id: 'mail',
    name: 'Gmail + Outlook',
    mark: '@',
    tone: 'mail',
    category: 'Sync',
    method: 'Gmail API / Microsoft Graph',
    summary: 'Ground relationships in the conversations you actually had, without re-exporting.',
    input: 'Mail data within the account permissions you explicitly grant.',
    output: 'Interaction history for recency, frequency and reciprocity.',
    availability: 'Live sync is not built. Import an mbox (Google Takeout) or ICS export on the Sources page today.',
    url: 'https://developers.google.com/workspace/gmail/api/guides',
    status: 'Planned · import exports today',
  },
  {
    id: 'nylas',
    name: 'Nylas',
    mark: 'N',
    tone: 'nylas',
    category: 'Sync',
    method: 'Unified communications API',
    summary: 'One route to email, calendars and contacts across providers.',
    input: 'An explicitly authorized account connection through Nylas.',
    output: 'Normalized account data and events.',
    availability: 'Not built. An optional convenience service with its own costs and data handling.',
    url: 'https://developer.nylas.com/',
    status: 'Planned (optional)',
  },
  {
    id: 'pdl',
    name: 'People Data Labs',
    mark: 'pd',
    tone: 'pdl',
    category: 'Enrichment',
    method: 'Person enrichment API · your own key',
    summary: 'Resolve a known person and add structured professional context.',
    input: 'Identity fields supplied for a one-to-one profile match.',
    output: 'Matched professional profile fields, with matching uncertainty preserved.',
    availability: 'Reserved as enrich.backend = "ownkeys"; the adapter is not built yet and fails clearly if selected.',
    url: 'https://docs.peopledatalabs.com/',
    status: 'Own-key adapter planned',
  },
  {
    id: 'hunter',
    name: 'Hunter',
    mark: 'H',
    tone: 'hunter',
    category: 'Enrichment',
    method: 'Email Finder / Verifier · your own key',
    summary: 'Find a work email and inspect its deliverability signals.',
    input: 'Name and company domain for discovery, or an existing email for verification.',
    output: 'A work email or a verification status, including uncertain outcomes.',
    availability: 'Reserved as enrich.backend = "ownkeys"; the adapter is not built yet.',
    url: 'https://hunter.io/api',
    status: 'Own-key adapter planned',
  },
];

const CATEGORIES = ['All', 'Introductions', 'Enrichment', 'Discovery', 'Sync'] as const;

export function Integrations() {
  const { config } = useApp();
  const backend = String(configValue(config, 'enrich.backend') ?? 'none');
  const tregOn = backend === 'treg';
  const [category, setCategory] = useState<(typeof CATEGORIES)[number]>('All');
  const [details, setDetails] = useState<Provider | null>(null);

  const statusOf = (p: Provider) => (p.id === 'treg' ? (tregOn ? 'Enabled in your config' : 'Available · off') : p.status!);
  const list = PROVIDERS.filter((p) => category === 'All' || p.category === category);

  return (
    <>
      <PageHead
        eyebrow="YOUR GRAPH. A WIDER WORLD."
        title="Bring the right help."
        sub="Discover someone new. Add missing context. Ask for an introduction."
      />
      <section className="integration-hero">
        <div>
          <span className="tag">LOCAL FIRST · EXTERNAL BY CHOICE</span>
          <h2>
            One workflow.
            <br />
            More ways forward.
          </h2>
          <p>Your own network comes first. Reach beyond it when you choose, with a clear view of what leaves and what it costs.</p>
          <div className="integration-actions">
            <a className="btn primary" href="#/playbooks/path">
              Find a path to someone <Icon name="arrow" />
            </a>
            <a className="btn" href="#/egress">
              See what has left
            </a>
          </div>
        </div>
        <div className="integration-flow">
          {(
            [
              ['01', 'Search locally', 'Your records + evidence · free'],
              ['02', 'Fill a gap', tregOn ? 'treg · quote first' : 'treg · off until you enable it'],
              ['03', 'Make the ask', 'Your connector · you send it'],
              ['04', 'Track the outcome', 'Introductions board · manual'],
            ] as const
          ).map(([n, t, d]) => (
            <div key={n}>
              <span>{n}</span>
              <section>
                <strong>{t}</strong>
                <small>{d}</small>
              </section>
            </div>
          ))}
        </div>
      </section>
      <div className="integration-status">
        <Icon name="shield" />
        {tregOn ? (
          <span>
            <strong>treg enrichment is enabled in your config.</strong> Every request is quoted first and sent only when you
            press Spend; each one is recorded in the egress log. No other external account is connected.
          </span>
        ) : (
          <span>
            <strong>No external account is connected.</strong> Search, playbooks and briefs run locally. No lookup, email or
            charge happens unless you enable an enricher and approve a quote.
          </span>
        )}
      </div>
      <div className="integration-filter" role="group" aria-label="Filter integrations">
        {CATEGORIES.map((c) => (
          <button type="button" key={c} aria-pressed={c === category} onClick={() => setCategory(c)}>
            {c}
          </button>
        ))}
      </div>
      <div className="provider-grid">
        {list.map((p) => (
          <article className="provider-card" key={p.id}>
            <div className="provider-top">
              <span className={`provider-mark ${p.tone}`} aria-hidden="true">
                {p.mark}
              </span>
              <span className={`tag ${p.id === 'treg' && tregOn ? '' : 'neutral'}`}>{statusOf(p)}</span>
            </div>
            <small className="provider-category">
              {p.category} / {p.method}
            </small>
            <h2>{p.name}</h2>
            <p>{p.summary}</p>
            <div className="provider-actions">
              {p.id === 'treg' && (
                <a className="btn" href="#/playbooks">
                  Open playbooks <Icon name="arrow" />
                </a>
              )}
              <button type="button" className="btn text" aria-label={`${p.name} details`} onClick={() => setDetails(p)}>
                Details
              </button>
            </div>
          </article>
        ))}
      </div>
      <section className="integration-contract">
        <div>
          <span className="eyebrow">THE OPENSTANCE CONTRACT</span>
          <h2>You decide what leaves.</h2>
        </div>
        <div>
          <h3>Only selected fields</h3>
          <p>Private notes and the full graph stay on your device. The quote lists every outbound request before it runs.</p>
        </div>
        <div>
          <h3>A budget, not a surprise</h3>
          <p>Each run is capped by enrich.max_usd_per_run and by the quote you approve.</p>
        </div>
        <div>
          <h3>Evidence stays attached</h3>
          <p>A public profile, an imported conversation and an accepted introduction are different signals, and are labeled so.</p>
        </div>
      </section>
      <p className="note">Independent integration roadmap, not an endorsement or partnership.</p>
      {details && (
        <Dialog title={details.name} onClose={() => setDetails(null)}>
          <div className="provider-top">
            <span className={`provider-mark ${details.tone}`} aria-hidden="true">
              {details.mark}
            </span>
            <span className="tag neutral">{statusOf(details)}</span>
          </div>
          <p>{details.summary}</p>
          <div className="integration-detail">
            <h3>What would be sent</h3>
            <p>{details.input}</p>
            <h3>What comes back</h3>
            <p>{details.output}</p>
            <h3>Current availability</h3>
            <p>{details.availability}</p>
          </div>
          <div className="modal-actions">
            <a className="btn" href={details.url} target="_blank" rel="noopener noreferrer">
              Official site <Icon name="external" />
            </a>
          </div>
        </Dialog>
      )}
    </>
  );
}
