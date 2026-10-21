import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { renderApp } from '../test/render';
import { parseHash } from './router';

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
