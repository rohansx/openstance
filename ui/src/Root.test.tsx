import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { TOKEN_KEY } from './api/token';
import { Root } from './Root';
import { defaultRoutes, fx } from './test/fixtures';
import { mockApi } from './test/mockApi';

describe('Root: token handoff and shell', () => {
  it('takes #token= from the URL, strips it, and uses it as the bearer token', async () => {
    window.history.replaceState(null, '', '/#token=abc123');
    const api = mockApi(defaultRoutes());
    render(<Root />);

    await screen.findByRole('navigation', { name: 'Main navigation' });
    expect(window.location.hash).toBe('#/');
    expect(window.location.href).not.toContain('abc123');
    expect(sessionStorage.getItem(TOKEN_KEY)).toBe('abc123');
    await waitFor(() => expect(api.callsTo('GET /stats')).toHaveLength(1));
    expect(api.callsTo('GET /stats')[0]!.headers.get('authorization')).toBe('Bearer abc123');
  });

  it('without a token shows the connect screen and makes no authenticated calls', async () => {
    window.history.replaceState(null, '', '/');
    const api = mockApi(defaultRoutes());
    render(<Root />);
    expect(await screen.findByRole('heading', { name: /connect this tab/i })).toBeInTheDocument();
    expect(api.calls.every((c) => c.path === '/health')).toBe(true);
    expect(screen.getByText(/openstance serve/)).toBeInTheDocument();
  });

  it('accepts a pasted token', async () => {
    window.history.replaceState(null, '', '/');
    mockApi(defaultRoutes());
    render(<Root />);
    await userEvent.type(await screen.findByLabelText(/api token/i), 'pasted-token');
    await userEvent.click(screen.getByRole('button', { name: /connect/i }));
    await screen.findByRole('navigation', { name: 'Main navigation' });
    expect(sessionStorage.getItem(TOKEN_KEY)).toBe('pasted-token');
  });

  it('a 401 forgets the token and returns to the connect screen', async () => {
    window.history.replaceState(null, '', '/#token=stale');
    mockApi({ ...defaultRoutes(), 'GET /stats': { status: 401, body: fx.unauthorized() } });
    render(<Root />);
    expect(await screen.findByRole('heading', { name: /connect this tab/i })).toBeInTheDocument();
    expect(sessionStorage.getItem(TOKEN_KEY)).toBeNull();
  });

  it('renders the prototype shell: workspace card, breadcrumb, device pill, import button, counts', async () => {
    window.history.replaceState(null, '', '/#token=t');
    mockApi(defaultRoutes());
    render(<Root />);
    const nav = await screen.findByRole('navigation', { name: 'Main navigation' });
    for (const label of [
      'Search people',
      'People',
      'Saved views',
      'Playbooks',
      'Shortlist',
      'Introductions',
      'Sources',
      'Review queue',
      'Integrations',
      'Agent access',
      'Egress log',
      'Settings',
    ]) {
      expect(within(nav).getByRole('link', { name: new RegExp(`^${label}`) })).toBeInTheDocument();
    }
    expect(screen.getByText('Stored on this device')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /import contacts/i })).toHaveAttribute('href', '#/sources');
    // counts come from GET /stats
    expect(await within(nav).findByText('1,210')).toBeInTheDocument();
    expect(within(nav).getByRole('link', { name: /^Review queue/ })).toHaveTextContent('2');
    expect(screen.getByLabelText('Breadcrumb')).toHaveTextContent('Search people');
  });

  it('navigates by hash and marks the current page', async () => {
    window.history.replaceState(null, '', '/#token=t');
    mockApi(defaultRoutes());
    render(<Root />);
    const nav = await screen.findByRole('navigation', { name: 'Main navigation' });
    await userEvent.click(within(nav).getByRole('link', { name: /^Review queue/ }));
    await waitFor(() => expect(window.location.hash).toBe('#/review'));
    expect(within(nav).getByRole('link', { name: /^Review queue/ })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByLabelText('Breadcrumb')).toHaveTextContent('Review queue');
  });
});
