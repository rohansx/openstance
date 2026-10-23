import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Icon } from './Icon';

type Surface = 'cli' | 'mcp' | 'web';

const M = ({ children }: { children: ReactNode }) => <span className="t-muted">{children}</span>;
const S = ({ children }: { children: ReactNode }) => <span className="t-string">{children}</span>;
const P = () => <span className="t-muted">$ </span>;

interface Sample {
  label: string;
  status: string;
  body: ReactNode;
}

// Output shapes are trimmed from the README quickstart and docs/tech-spec/interfaces.md,
// run against the fictional demo dataset in fixtures/demo/.
const samples: Record<Surface, Sample> = {
  cli: {
    label: 'CLI · works today',
    status: 'Shipped',
    body: (
      <>
        <P />ost search <S>"top 5 fintech founders who replied"</S>
        {'\n'}
        <M>  #  person       title / company           score</M>
        {'\n'}  1  Maya Chen    Co-founder &amp; CEO, Ledgerline  0.81
        {'\n'}
        <M>       2026-10-09  email out  "Re: Fintech founders…"</M>
        {'\n'}  2  Arjun Mehta  Founder &amp; CEO, Tallyfin     0.56
        {'\n'}
        <M>  …</M>
        {'\n'}11 results (5 shown, by score)
        {'\n'}
        <M>filters</M> title=[founder] industry=[fintech] replied=true
        {'\n'}
        <M>rerun  </M> openstance search --title founder --replied
        {'\n\n'}
        <S>Every result carries the evidence it was found on.</S>
      </>
    ),
  },
  mcp: {
    label: 'MCP · stdio server, works today',
    status: 'Shipped',
    body: (
      <>
        <P />claude mcp add openstance -- openstance mcp
        {'\n\n'}
        <M>tool</M>  search_people  <M>(local · read-only · free)</M>
        {'\n'}
        {'{ '}
        <S>"text"</S>: "fintech founders who replied", <S>"limit"</S>: 5 {'}'}
        {'\n\n'}
        <M>→ structuredContent</M>
        {'\n'}
        {'{ '}
        <S>"v"</S>: 1,
        {'\n  '}
        <S>"query"</S>: {'{ '}
        <S>"title"</S>: ["founder"], <S>"replied"</S>: true {'}'},
        {'\n  '}
        <S>"total"</S>: 11,
        {'\n  '}
        <S>"results"</S>: [ {'{ '}
        <S>"rank"</S>: 1, <S>"person"</S>: {'{ … }'}, <S>"evidence"</S>: [ … ] {'}'} ] {'}'}
        {'\n\n'}
        <M>Same graph. Same JSON as the CLI's --json. Your agent.</M>
      </>
    ),
  },
  web: {
    label: 'WEB · local UI, next',
    status: 'Arriving',
    body: (
      <>
        <P />openstance serve
        {'\n'}
        <M>openstance api</M>  http://127.0.0.1:7411
        {'\n'}
        <M>web ui        </M>  http://127.0.0.1:7411/#token=Zk3…
        {'\n'}
        <M>mcp           </M>  http://127.0.0.1:7411/mcp
        {'\n\n'}
        <M>01</M> Search with filters you can edit as chips
        {'\n'}
        <M>02</M> Open a person: fields, sources, score, paths
        {'\n'}
        <M>03</M> Review uncertain merges
        {'\n'}
        <M>04</M> Run a playbook behind its quote
        {'\n\n'}
        <S>Loopback only. Bearer token. No cookies.</S>
      </>
    ),
  },
};

const order: Surface[] = ['cli', 'mcp', 'web'];

export default function TerminalCard() {
  const [surface, setSurface] = useState<Surface>('cli');
  const tabs = useRef<Record<Surface, HTMLButtonElement | null>>({ cli: null, mcp: null, web: null });
  const base = useId();
  const tabId = (s: Surface) => `${base}-tab-${s}`;
  const panelId = `${base}-panel`;
  const sample = samples[surface];

  function select(next: Surface) {
    setSurface(next);
    tabs.current[next]?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    const i = order.indexOf(surface);
    const moves: Record<string, number> = {
      ArrowRight: (i + 1) % order.length,
      ArrowLeft: (i - 1 + order.length) % order.length,
      Home: 0,
      End: order.length - 1,
    };
    const target = moves[event.key];
    if (target === undefined) return;
    event.preventDefault();
    select(order[target]!);
  }

  return (
    <div className="terminal">
      <div className="terminal-top">
        <span className="window-dots" aria-hidden="true">
          ● ● ●
        </span>
        <span>{sample.label}</span>
        <span className={`terminal-status ${surface === 'web' ? 'is-soon' : ''}`}>{sample.status}</span>
      </div>
      <div className="surface-tabs" role="tablist" aria-label="Product surfaces">
        {order.map((s) => (
          <button
            key={s}
            ref={(el) => {
              tabs.current[s] = el;
            }}
            type="button"
            role="tab"
            id={tabId(s)}
            aria-selected={surface === s}
            aria-controls={panelId}
            tabIndex={surface === s ? 0 : -1}
            className={surface === s ? 'selected' : undefined}
            onClick={() => setSurface(s)}
            onKeyDown={onKeyDown}
          >
            {s.toUpperCase()}
          </button>
        ))}
      </div>
      <pre role="tabpanel" id={panelId} aria-labelledby={tabId(surface)} tabIndex={0}>
        {sample.body}
      </pre>
      {surface === 'web' && (
        <p className="terminal-note">
          <Icon name="clock" /> <code>openstance serve</code> is arriving in this release. The commands above it work today.
        </p>
      )}
    </div>
  );
}
