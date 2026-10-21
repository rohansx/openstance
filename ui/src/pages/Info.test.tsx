import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { TOKEN_KEY } from '../api/token';
import { fx } from '../test/fixtures';
import { renderApp } from '../test/render';

const withBackend = (backend: string) => {
  const c = fx.config();
  (c.config as { enrich: { backend: string } }).enrich.backend = backend;
  return c;
};

const card = (name: string) => screen.getByRole('heading', { name }).closest('article')!;

describe('Integrations', () => {
  it('never shows an integration as connected; roadmap items say Planned', async () => {
    renderApp('#/integrations');
    await screen.findByRole('heading', { name: 'treg' });
    expect(within(card('treg')).getByText(/available · off/i)).toBeInTheDocument();
    for (const name of ['Boardy', 'Exa', 'Gmail + Outlook', 'Nylas']) {
      expect(within(card(name)).getByText(/planned/i)).toBeInTheDocument();
    }
    for (const name of ['People Data Labs', 'Hunter']) {
      expect(within(card(name)).getByText(/own-key adapter planned/i)).toBeInTheDocument();
    }
    expect(screen.getByText(/no external account is connected/i)).toBeInTheDocument();
    expect(screen.queryAllByText(/^connected$/i)).toHaveLength(0);
  });

  it('treg shows as enabled (not connected) when the config turns it on', async () => {
    renderApp('#/integrations', { 'GET /config': withBackend('treg') });
    await waitFor(() => expect(within(card('treg')).getByText(/enabled in your config/i)).toBeInTheDocument());
    expect(screen.getByText(/every request is quoted first/i)).toBeInTheDocument();
  });

  it('filters by category and opens provider details', async () => {
    renderApp('#/integrations');
    await screen.findByRole('heading', { name: 'treg' });
    await userEvent.click(screen.getByRole('button', { name: 'Sync' }));
    expect(screen.getByRole('button', { name: 'Sync' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.queryByRole('heading', { name: 'treg' })).toBeNull();
    expect(screen.getByRole('heading', { name: 'Nylas' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'All' }));
    await userEvent.click(within(card('treg')).getByRole('button', { name: /details/i }));
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByText(/what would be sent/i)).toBeInTheDocument();
    expect(within(dialog).getByText(/OPENSTANCE_TREG_TOKEN/)).toBeInTheDocument();
  });
});

describe('Agent access', () => {
  it('shows stdio and HTTP MCP configs without revealing the token', async () => {
    renderApp('#/agents');
    expect(await screen.findByText('claude mcp add openstance -- openstance mcp')).toBeInTheDocument();
    expect(screen.getByText(/"command": "openstance"/)).toBeInTheDocument();
    expect(screen.getByText(/"url": "http:\/\/localhost(:\d+)?\/mcp"/)).toBeInTheDocument();
    expect(screen.getByText(/Bearer <token from ~\/\.openstance\/api\.token>/)).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('test-token');
    expect(screen.getByText(/cannot reach a loopback address/i)).toBeInTheDocument();
    const tools = screen.getByRole('table', { name: /mcp tools/i });
    for (const t of ['search_people', 'get_person', 'how_connected', 'path_to', 'brief', 'list_views', 'search_view', 'quote_enrich']) {
      expect(within(tools).getByText(t)).toBeInTheDocument();
    }
  });
});

describe('Settings', () => {
  it('shows the effective config, version and can forget the token', async () => {
    renderApp('#/settings');
    const cfg = await screen.findByRole('region', { name: /effective config/i });
    expect(await within(cfg).findByText('enrich.backend')).toBeInTheDocument();
    expect(within(cfg).getByText('model.parser')).toBeInTheDocument();
    expect(screen.getByText(/~\/\.openstance\/config\.toml/)).toBeInTheDocument();
    expect(await screen.findByText(/v0\.4\.0 · schema 3/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: /forget the token in this tab/i }));
    expect(await screen.findByRole('heading', { name: /connect this tab/i })).toBeInTheDocument();
    expect(sessionStorage.getItem(TOKEN_KEY)).toBeNull();
  });
});
