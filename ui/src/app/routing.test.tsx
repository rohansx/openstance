import { act, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderApp } from '../test/render';
import { parseHash } from './router';

function go(hash: string) {
  act(() => {
    window.history.replaceState(null, '', `/${hash}`);
    window.dispatchEvent(new HashChangeEvent('hashchange'));
  });
}

describe('routing: unknown routes (U7)', () => {
  it('parses an unknown hash as not found, and the empty hash as search', () => {
    expect(parseHash('#/nope')).toEqual({ name: 'notfound', path: 'nope' });
    expect(parseHash('#/people/812/extra')).toEqual({ name: 'person', id: '812' });
    expect(parseHash('')).toEqual({ name: 'discover' });
    expect(parseHash('#/')).toEqual({ name: 'discover' });
    expect(parseHash('#')).toEqual({ name: 'discover' });
  });

  it('renders a not-found page with a link back to search instead of the search page', async () => {
    renderApp('#/does-not-exist');
    expect(await screen.findByRole('heading', { name: /page not found/i })).toBeInTheDocument();
    expect(screen.getByText('#/does-not-exist')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /back to search/i })).toHaveAttribute('href', '#/discover');
    expect(screen.queryByRole('searchbox', { name: /search your network/i })).toBeNull();
  });
});

describe('routing: scroll and focus on route change (U3)', () => {
  afterEach(() => vi.restoreAllMocks());

  it('scrolls to the top and focuses main when the route changes', async () => {
    const scrollTo = vi.fn();
    vi.stubGlobal('scrollTo', scrollTo);
    renderApp('#/people/812');
    await screen.findByRole('heading', { name: 'Tom Okafor', level: 1 });
    scrollTo.mockClear();

    go('#/playbooks/brief?person=812');
    await screen.findByRole('region', { name: /run brief/i });
    expect(scrollTo).toHaveBeenCalledWith(0, 0);
    expect(document.activeElement).toBe(document.getElementById('main'));
  });
});
