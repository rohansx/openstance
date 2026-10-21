import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import App from './App';

describe('App', () => {
  it('renders the hero heading as the only h1', () => {
    render(<App />);
    const h1 = screen.getByRole('heading', { level: 1 });
    expect(h1).toHaveTextContent(/Your network,\s*as a file\./);
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
  });

  it('has the required sections as landmarks reachable from the nav', () => {
    render(<App />);
    for (const id of ['how-it-works', 'architecture', 'open-core', 'install']) {
      expect(document.getElementById(id)).not.toBeNull();
    }
    const nav = screen.getByRole('navigation', { name: /primary/i });
    expect(within(nav).getByRole('link', { name: /how it works/i })).toHaveAttribute('href', '#how-it-works');
    expect(within(nav).getByRole('link', { name: /install/i })).toHaveAttribute('href', '#install');
  });

  it('shows the four-step strip', () => {
    render(<App />);
    const strip = screen.getByRole('list', { name: /principles/i });
    expect(within(strip).getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      'Export first.',
      'Search locally.',
      'Inspect the evidence.',
      'Choose what leaves.',
    ]);
  });

  it('gives an install snippet with the real repository', () => {
    render(<App />);
    const install = document.getElementById('install');
    expect(install).not.toBeNull();
    expect(install!.textContent).toContain(
      'cargo install --git https://github.com/rohansx/openstance openstance',
    );
    expect(install!.textContent).toContain('claude mcp add openstance -- openstance mcp');
  });

  it('labels cloud pricing as a proposal and keeps local free', () => {
    render(<App />);
    const core = document.getElementById('open-core')!;
    expect(core.textContent).toMatch(/\$0/);
    expect(core.textContent).toMatch(/proposal/i);
    expect(core.textContent).toMatch(/indicative/i);
    expect(core.textContent).toMatch(/no search metering/i);
  });

  it('says honestly that the web UI is arriving, not shipped', () => {
    render(<App />);
    expect(screen.getAllByText(/arriving in this release/i).length).toBeGreaterThan(0);
  });

  it('only links to in-page anchors or the project on GitHub', () => {
    render(<App />);
    const hrefs = screen.getAllByRole('link').map((a) => a.getAttribute('href') ?? '');
    expect(hrefs.length).toBeGreaterThan(0);
    for (const href of hrefs) {
      expect(href.startsWith('#') || href.startsWith('https://github.com/rohansx/openstance')).toBe(true);
    }
  });

  it('opens external links without leaking the opener or referrer', () => {
    render(<App />);
    for (const a of screen.getAllByRole('link')) {
      if (a.getAttribute('href')?.startsWith('https://')) {
        expect(a.getAttribute('rel')).toMatch(/noopener/);
        expect(a.getAttribute('rel')).toMatch(/noreferrer/);
      }
    }
  });
});
