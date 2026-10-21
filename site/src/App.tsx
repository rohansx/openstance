import type { ReactNode } from 'react';
import { Icon, Logo, type IconName } from './Icon';
import InstallSnippet from './InstallSnippet';
import TerminalCard from './TerminalCard';
import { ARCHITECTURE_DOC, DOCS, MCP_GUIDE, PLAYBOOKS, PRICING_DOC, README, REPO } from './links';

function Ext({ href, className, children }: { href: string; className?: string; children: ReactNode }) {
  return (
    <a href={href} className={className} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  );
}

// From `ost who "Maya Chen"` on the fictional demo dataset (README quickstart, step 5).
const scoreComponents: [string, number, string][] = [
  ['Recency', 0.98, '5 days since last contact'],
  ['Frequency', 0.961, '65 interactions'],
  ['Reciprocity', 1, '22 replies, 7 threads started'],
  ['Meetings', 0, 'no meetings imported'],
  ['Breadth', 0.5, 'email, linkedin'],
];

const playbooks: [IconName, string, string, string][] = [
  ['route', 'Find a warm path', '`how` ranks the strongest routes to someone in your graph, each with the threads behind it. `path` extends that to people outside it, behind a quote.', 'Available'],
  ['people', 'Ask who someone is', '`who` shows fields and where they came from, every score component with its weight, identities, recent interactions and mutuals.', 'Available'],
  ['mail', 'Write a brief', '`brief` gives a ten-line, fact-only Markdown brief where every line traces to an interaction or a logged lookup.', 'Available'],
  ['lock', 'Share a slice', '`view export` writes an age-encrypted .ost file with professional fields only. Never interactions, notes or do-not-share people.', 'Available'],
];

const faqs: [string, ReactNode][] = [
  [
    'Can I run it today?',
    <>
      Yes, from source. <code>cargo install</code> builds the <code>openstance</code> binary (alias <code>ost</code>)
      with the CLI, the importers, search, playbooks, views and the MCP server. You need Rust stable and a C
      compiler, because SQLite is compiled in. Prebuilt binaries, an install script and Homebrew are planned for
      v1.0.
    </>,
  ],
  [
    'Does my contact data leave my machine?',
    <>
      Not with the default config: no telemetry, no account, and CI runs every command with the network blocked.
      The one default download is the local embedding model (about 33 MB, nothing about you is sent); set{' '}
      <code>model.embedder = "none"</code> to skip it. Enrichment is off until you turn it on, and then every call
      is quoted first and written to <code>openstance enrich log</code>.
    </>,
  ],
  [
    'What does it read from my exports?',
    <>
      Email headers only, never bodies. LinkedIn names, profile URLs, companies, positions and dates, never message
      text. Calendar times, titles and attendees, never descriptions or locations. From the X archive, follows and
      direct-message metadata, never message text or tweets.
    </>,
  ],
  [
    'Can I use my own model?',
    <>
      Search uses local embeddings (bge-small-en-v1.5 on your CPU) or plain keyword search with no model at all.
      Bring-your-own local or hosted models for query parsing and reranking are part of the v1.0 work, and the parsed
      filters are always shown so you can edit and rerun without one.
    </>,
  ],
  [
    'Can I use it from claude.ai or ChatGPT?',
    <>
      Local MCP clients such as Claude Code, Cursor and Claude Desktop work today over stdio. Remote connectors
      cannot reach a loopback address, and openstance does not ship a tunnel. A hosted endpoint is a cloud-tier
      idea, not a product.
    </>,
  ],
  [
    'What happens to a view I shared?',
    <>
      The recipient imports the encrypted <code>.ost</code> file and searches it next to their own graph without
      merging it in. Encryption controls who can open it, not what they do afterwards: a copy that has been
      decrypted cannot be taken back.
    </>,
  ],
];

export default function App() {
  return (
    <div className="landing">
      <a className="skip" href="#main">
        Skip to content
      </a>
      <header className="land-nav">
        <a className="brand" href="#top" aria-label="openstance, back to top">
          <Logo />
          openstance
        </a>
        <nav aria-label="Primary">
          <a href="#how-it-works">How it works</a>
          <a href="#architecture">Architecture</a>
          <a href="#open-core">Open core</a>
          <a href="#install">Install</a>
        </nav>
        <Ext href={REPO} className="btn primary">
          <Icon name="code" /> GitHub
        </Ext>
      </header>

      <main id="main">
        <section className="land-hero" id="top" aria-labelledby="hero-title">
          <div className="hero-copy">
            <div className="land-kicker">
              <span className="dot" aria-hidden="true" /> OPEN SOURCE. LOCAL FIRST. YOURS.
            </div>
            <h1 id="hero-title">
              Your network,
              <br />
              <em>as a file.</em>
            </h1>
            <p>
              The people you know. The context you forget.{' '}
              <br />
              One place to find the connection that matters.
            </p>
            <p className="hero-sub">
              openstance reads the exports you can already download, works out who is who, scores each relationship,
              and lets you search the result in plain English from the terminal or from an agent. Everything lives in
              one SQLite file on your machine.
            </p>
            <div className="hero-actions">
              <a className="btn primary" href="#install">
                Install from source <Icon name="arrow" />
              </a>
              <Ext className="btn" href={REPO}>
                <Icon name="code" /> View the source
              </Ext>
            </div>
            <ul className="hero-proof" aria-label="Highlights">
              <li>
                <Icon name="check" /> Free local search
              </li>
              <li>
                <Icon name="check" /> No account or API key
              </li>
              <li>
                <Icon name="check" /> Export always
              </li>
            </ul>
          </div>

          <div className="hero-product">
            <TerminalCard />
            <div className="floating-result" aria-label="Example result">
              <div className="avatar" aria-hidden="true">
                MC
              </div>
              <div>
                <strong>Maya Chen</strong>
                <span>Co-founder &amp; CEO · Ledgerline</span>
                <small>
                  <Icon name="check" /> Replied 2026-10-09 · score 0.81
                </small>
              </div>
              <div className="result-stamp" aria-hidden="true">
                EVIDENCE
                <br />
                <b>VISIBLE</b>
              </div>
            </div>
            <p className="illustration-label">Fictional demo dataset (fixtures/demo) · output trimmed</p>
          </div>
        </section>

        <div className="principle-strip">
          <span>YOUR DATA, YOUR FORMAT</span>
          <ol aria-label="Principles">
            <li>Export first.</li>
            <li>Search locally.</li>
            <li>Inspect the evidence.</li>
            <li>Choose what leaves.</li>
          </ol>
        </div>

        <section className="land-section" id="how-it-works" aria-labelledby="how-title">
          <div className="section-intro">
            <div className="land-kicker">01 / A LITTLE CONTEXT GOES A LONG WAY</div>
            <h2 id="how-title">
              You know more people
              <br />
              than you remember.
            </h2>
            <p>
              Years of email headers, calendar invites and connections already say who you know and who replies. Every
              result should explain who fits, how you are connected, and what the evidence actually says.
            </p>
          </div>
          <ol className="product-story">
            <li className="story-step">
              <span aria-hidden="true">01</span>
              <div>
                <h3>Bring what you already have.</h3>
                <p>
                  Import a Google Takeout mbox, LinkedIn's Connections.csv, a vCard address book, calendar .ics files
                  and the X archive. The file type is detected from the content, and re-importing is a no-op.
                </p>
              </div>
            </li>
            <li className="story-step">
              <span aria-hidden="true">02</span>
              <div>
                <h3>Find people. See why.</h3>
                <p>
                  Plain English becomes filters you can see and rerun. Keyword search plus local embeddings finds
                  "developer tools" when the title says "SDK". Uncertain merges wait in a review queue.
                </p>
              </div>
            </li>
            <li className="story-step">
              <span aria-hidden="true">03</span>
              <div>
                <h3>Put the connection to work.</h3>
                <p>
                  Ask <code>who</code> for the score behind a person, <code>how</code> for the warmest path in, and{' '}
                  <code>brief</code> for a fact-only summary. Share a slice as an encrypted view.
                </p>
              </div>
            </li>
          </ol>
        </section>

        <section className="evidence-showcase" aria-labelledby="evidence-title">
          <div>
            <div className="land-kicker">EVIDENCE OVER GUESSWORK</div>
            <h2 id="evidence-title">
              A score you can
              <br />
              actually question.
            </h2>
            <p>
              Recency. Frequency. Reciprocity. Meetings. Breadth. Five visible components with published formulas and
              presets for founders, recruiters, sellers and communities. Newsletters and no-reply senders score zero.
            </p>
            <p className="small-print">
              Values from <code>ost who "Maya Chen"</code> on the fictional demo dataset, default preset.
            </p>
          </div>
          <figure className="score-preview" aria-labelledby="score-caption">
            <div className="score-preview-head">
              <div className="avatar tone-1" aria-hidden="true">
                MC
              </div>
              <div>
                <strong>Maya Chen</strong>
                <span>Co-founder &amp; CEO · Ledgerline</span>
              </div>
              <span className="tag">score 0.81</span>
            </div>
            <dl>
              {scoreComponents.map(([label, value, why]) => (
                <div className="score-line" key={label}>
                  <dt>{label}</dt>
                  <dd className="bar" title={why}>
                    <svg viewBox="0 0 100 5" preserveAspectRatio="none" aria-hidden="true" focusable="false">
                      <rect width="100" height="5" rx="2.5" className="bar-track" />
                      <rect width={value * 100} height="5" rx="2.5" className="bar-fill" />
                    </svg>
                  </dd>
                  <dd className="value">{value.toFixed(2)}</dd>
                </div>
              ))}
            </dl>
            <figcaption className="source-note" id="score-caption">
              <Icon name="database" /> S = 0.35·R + 0.20·F + 0.25·C + 0.15·M + 0.05·B = 0.81
            </figcaption>
          </figure>
        </section>

        <section className="land-section architecture" id="architecture" aria-labelledby="arch-title">
          <div className="section-intro">
            <div className="land-kicker">02 / ONE GRAPH. EVERY SURFACE.</div>
            <h2 id="arch-title">
              A small core.
              <br />A lot of possibility.
            </h2>
            <p>
              One Rust binary. One SQLite file with full-text and vector search and a documented schema you can open
              with any SQLite tool. The CLI, the MCP server and the local web UI all return the same JSON.
            </p>
            <Ext className="inline-link" href={ARCHITECTURE_DOC}>
              Read the architecture <Icon name="arrow" />
            </Ext>
          </div>
          <figure className="architecture-diagram" aria-label="Architecture: exports flow into graph.db, which serves the CLI, MCP and web UI">
            <div className="diagram-label">SHIPPED TODAY · WEB UI ARRIVING IN THIS RELEASE</div>
            <ul className="file-inputs" aria-label="Importers">
              <li>Takeout.mbox</li>
              <li>Connections.csv</li>
              <li>contacts.vcf</li>
              <li>calendar.ics</li>
              <li>X archive.zip</li>
            </ul>
            <div className="diagram-arrow" aria-hidden="true">
              ↓
            </div>
            <div className="db-file">
              <Icon name="database" />
              <div>
                <strong>graph.db</strong>
                <span>SQLite · FTS5 · sqlite-vec · people, identities, interactions, evidence</span>
              </div>
              <small>ON YOUR DISK</small>
            </div>
            <div className="diagram-arrow" aria-hidden="true">
              ↓
            </div>
            <ul className="surface-row" aria-label="Surfaces">
              <li>$_ CLI</li>
              <li>⌘ MCP stdio</li>
              <li className="is-soon">
                ▤ Web UI <em>next</em>
              </li>
            </ul>
            <div className="egress-box">
              <Icon name="shield" />
              <div>
                <strong>Outside data is an explicit choice.</strong>
                <p>
                  Enrichment is off by default. When you turn it on: plan, show the data and the price, approve, run,
                  log. People marked no_enrich are never sent.
                </p>
              </div>
            </div>
          </figure>
        </section>

        <section className="playbook-section" aria-labelledby="playbooks-title">
          <div className="land-kicker">03 / GO FROM “WHO?” TO “WHAT NEXT?”</div>
          <div className="section-title-row">
            <h2 id="playbooks-title">
              Small playbooks.
              <br />
              Useful next steps.
            </h2>
            <Ext className="btn" href={PLAYBOOKS}>
              Read the agent skills <Icon name="arrow" />
            </Ext>
          </div>
          <div className="land-playbooks">
            {playbooks.map(([icon, title, text, status]) => (
              <article key={title}>
                <Icon name={icon} />
                <h3>{title}</h3>
                <p>{renderCode(text)}</p>
                <small>{status}</small>
              </article>
            ))}
          </div>
          <p className="roadmap-note">
            <Icon name="clock" /> In progress for v1.0: <code>reconnect</code>, <code>accounts</code>,{' '}
            <code>watch</code>, <code>refresh</code> and <code>verify</code>.
          </p>
        </section>

        <section className="ownership-section" id="open-core" aria-labelledby="core-title">
          <div>
            <div className="land-kicker">04 / YOUR NETWORK SHOULD STAY YOURS</div>
            <h2 id="core-title">
              Search is the core.
              <br />
              Ownership is the point.
            </h2>
            <p>
              Search, the file, every importer, scoring, playbooks, MCP, the CLI and the HTTP API are free forever.
              A future cloud tier would sell only what is painful to self-host: encrypted sync, hosted endpoints,
              scheduled playbooks and team administration.
            </p>
            <ul className="ownership-points">
              <li>
                <Icon name="check" /> No search metering, ever
              </li>
              <li>
                <Icon name="check" /> Export is never paywalled
              </li>
              <li>
                <Icon name="check" /> No pooled contact database
              </li>
              <li>
                <Icon name="check" /> Enrichment with your own key, no markup
              </li>
            </ul>
          </div>
          <div className="release-card">
            <span className="tag">PRICING</span>
            <h3>Local is free. Cloud is a proposal.</h3>
            <table className="price-table">
              <caption className="visually-hidden">Plans</caption>
              <tbody>
                <tr>
                  <th scope="row">Local</th>
                  <td>$0 forever</td>
                </tr>
                <tr>
                  <th scope="row">Individual cloud</th>
                  <td>$8 / month, indicative</td>
                </tr>
                <tr>
                  <th scope="row">Team</th>
                  <td>$12 / seat / month, indicative</td>
                </tr>
                <tr>
                  <th scope="row">Enrichment</th>
                  <td>Pay as you go</td>
                </tr>
              </tbody>
            </table>
            <p className="small-print">
              The cloud tier is a proposal, not a purchasable service. Nothing in it is built, and it is gated on
              users asking for sync, hosted MCP or teams. Prices are not yet backed by research.
            </p>
            <Ext className="btn primary" href={PRICING_DOC}>
              Read the open-core rules <Icon name="arrow" />
            </Ext>
          </div>
        </section>

        <section className="install-section" id="install" aria-labelledby="install-title">
          <div className="section-intro">
            <div className="land-kicker">05 / FIVE COMMANDS</div>
            <h2 id="install-title">
              Install it.
              <br />
              Import. Search.
            </h2>
            <p>
              Build from source with Rust stable and a C compiler (Xcode Command Line Tools on macOS,
              build-essential on Linux). Then point it at your exports.
            </p>
            <Ext className="inline-link" href={MCP_GUIDE}>
              Set up Cursor, Claude Desktop and other MCP clients <Icon name="arrow" />
            </Ext>
          </div>
          <InstallSnippet />
        </section>

        <section className="faq-section" aria-labelledby="faq-title">
          <div className="land-kicker">A FEW THINGS WORTH KNOWING</div>
          <h2 id="faq-title">Built to be understandable.</h2>
          {faqs.map(([q, a]) => (
            <details key={q}>
              <summary>
                {q}
                <span aria-hidden="true">+</span>
              </summary>
              <p>{a}</p>
            </details>
          ))}
        </section>

        <section className="land-final" aria-labelledby="final-title">
          <div className="land-kicker">THE PEOPLE YOU KNOW. ON YOUR TERMS.</div>
          <h2 id="final-title">
            Make your network
            <br />
            work for you.
          </h2>
          <div className="hero-actions centered">
            <a className="btn primary" href="#install">
              Install openstance <Icon name="arrow" />
            </a>
            <Ext className="btn" href={DOCS}>
              Read the docs
            </Ext>
          </div>
          <span>Open source · local first · demo data is fictional</span>
        </section>
      </main>

      <footer className="land-footer">
        <a className="brand" href="#top" aria-label="openstance, back to top">
          <Logo />
          openstance
        </a>
        <p>Your network as a file. Your choices, all the way down.</p>
        <Ext href={REPO}>GitHub ↗</Ext>
        <Ext href={DOCS}>Docs ↗</Ext>
        <Ext href={README}>README ↗</Ext>
        <span>Apache-2.0 · working name</span>
      </footer>
    </div>
  );
}

// Render `code` spans in short copy strings.
function renderCode(text: string): ReactNode[] {
  return text.split(/(`[^`]+`)/).map((part, i) =>
    part.startsWith('`') ? <code key={i}>{part.slice(1, -1)}</code> : part,
  );
}
