import { PageHead } from '../components/bits';
import { Icon } from '../components/Icon';
import { useApp } from '../app/context';

const TOOLS: [string, string, string][] = [
  ['search_people', 'Search your graph with visible filters and dated evidence.', 'read-only'],
  ['get_person', 'Identities, interactions, mutuals and score components.', 'read-only'],
  ['how_connected', 'The top paths to someone in your graph, with evidence for each edge.', 'read-only'],
  ['path_to', 'Warm paths to anyone; quotes before any lookup.', 'may spend · quote first'],
  ['brief', 'A short, source-backed brief; optional web search is quoted.', 'may spend · quote first'],
  ['list_views', 'The views you own or imported.', 'read-only'],
  ['search_view', 'Search inside an imported view.', 'read-only'],
  ['quote_enrich', 'Price enrichment requests without sending contact data.', 'prices only'],
];

function Snippet({ title, code }: { title: string; code: string }) {
  const { toast } = useApp();
  return (
    <div className="snippet">
      <h3>{title}</h3>
      <pre>{code}</pre>
      <button
        type="button"
        className="btn"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(code);
            toast('Copied.');
          } catch {
            toast('Select the text and copy it manually.');
          }
        }}
      >
        <Icon name="code" />
        Copy
      </button>
    </div>
  );
}

export function Agents() {
  const origin = window.location.origin.startsWith('http') ? window.location.origin : 'http://127.0.0.1:7411';
  const http = JSON.stringify(
    {
      mcpServers: {
        openstance: { url: `${origin}/mcp`, headers: { Authorization: 'Bearer <token from ~/.openstance/api.token>' } },
      },
    },
    null,
    2,
  );
  const cursor = JSON.stringify({ mcpServers: { openstance: { command: 'openstance', args: ['mcp'] } } }, null, 2);
  return (
    <>
      <PageHead
        eyebrow="ONE GRAPH. THE SAME ANSWERS EVERYWHERE."
        title="Your assistant’s people layer."
        sub="The CLI, MCP and this HTTP API return the same JSON. Give your agent the graph without giving it your inbox."
      />
      <div className="section-grid">
        <section className="panel agent-reference">
          <span className="eyebrow">LOCAL AGENTS · STDIO</span>
          <Snippet title="Claude Code" code="claude mcp add openstance -- openstance mcp" />
          <Snippet title="Cursor (~/.cursor/mcp.json)" code={cursor} />
          <p className="note">
            Add <code>--read-only</code> to <code>openstance mcp</code> to hide every tool that could spend money.
          </p>
        </section>
        <section className="panel">
          <div className="panel-icon">
            <Icon name="graph" />
          </div>
          <h2>MCP over HTTP</h2>
          <p>
            For local clients that speak streamable HTTP, while <code>openstance serve</code> runs. Same bearer token as this
            page, and people marked “Do not share” are left out of every result.
          </p>
          <Snippet title="Client config" code={http} />
          <p className="note">
            The token is in <code>~/.openstance/api.token</code>; it is not shown here. Remote connectors (claude.ai, ChatGPT)
            cannot reach a loopback address. openstance ships no tunnel; exposing your graph to the internet is a deliberate
            step with its own risks.
          </p>
        </section>
      </div>
      <h3 className="subtle-label">MCP tools</h3>
      <div className="panel table-wrap">
        <table aria-label="MCP tools">
          <thead>
            <tr>
              <th>Tool</th>
              <th>What it does</th>
              <th>Money</th>
            </tr>
          </thead>
          <tbody>
            {TOOLS.map(([t, d, m]) => (
              <tr key={t}>
                <td>
                  <code>{t}</code>
                </td>
                <td>{d}</td>
                <td>
                  <span className={`tag ${m.startsWith('may') ? 'warn' : 'neutral'}`}>{m}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
