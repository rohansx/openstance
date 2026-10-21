import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Markdown } from './markdown';

const BRIEF = `**Delia Quintero** - Engineering Manager, Corvid Analytics (score 0.57)
- How you know them: 70 interactions since 2023-02-26 (email, linkedin).
- Last contact: 2026-10-14 email "Re: Corvid Analytics: Renewal discussion".
- Best mutual: Jordan Avery (score 0.56, 69 shared email, strength 0.56).`;

describe('safe markdown renderer (U4)', () => {
  it('renders the brief format: a bold lead line and a bullet list', () => {
    const { container } = render(<Markdown source={BRIEF} />);
    expect(screen.getByText('Delia Quintero').tagName).toBe('STRONG');
    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(3);
    expect(items[0]).toHaveTextContent('How you know them: 70 interactions since 2023-02-26 (email, linkedin).');
    expect(container.querySelector('p')).toHaveTextContent('Delia Quintero - Engineering Manager, Corvid Analytics (score 0.57)');
  });

  it('headings, ordered lists, inline code, emphasis, quotes and fenced code', () => {
    const { container } = render(
      <Markdown source={'## Mutuals\n\n1. first\n2. second\n\nUse `openstance brief` and *really* _do_ it.\n\n> quoted\n\n```\nraw <b>code</b>\n```'} />,
    );
    expect(screen.getByRole('heading', { name: 'Mutuals' })).toBeInTheDocument();
    expect(container.querySelector('ol')!.children).toHaveLength(2);
    expect(screen.getByText('openstance brief').tagName).toBe('CODE');
    expect(screen.getByText('really').tagName).toBe('EM');
    expect(screen.getByText('do').tagName).toBe('EM');
    expect(container.querySelector('blockquote')).toHaveTextContent('quoted');
    expect(container.querySelector('pre')).toHaveTextContent('raw <b>code</b>');
    expect(container.querySelector('pre b')).toBeNull();
  });

  it('never renders raw HTML: tags stay text and no elements are created', () => {
    const { container } = render(<Markdown source={'<script>alert(1)</script>\n- <img src=x onerror=alert(1)> **ok**'} />);
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    expect(container).toHaveTextContent('<script>alert(1)</script>');
    expect(container).toHaveTextContent('<img src=x onerror=alert(1)> ok');
  });

  it('links only for http(s) and mailto; other schemes stay text', () => {
    render(<Markdown source={'[site](https://example.org/a) [bad](javascript:alert(1)) [mail](mailto:a@b.example)'} />);
    const site = screen.getByRole('link', { name: 'site' });
    expect(site).toHaveAttribute('href', 'https://example.org/a');
    expect(site).toHaveAttribute('rel', 'noopener noreferrer');
    expect(screen.getByRole('link', { name: 'mail' })).toHaveAttribute('href', 'mailto:a@b.example');
    expect(screen.queryByRole('link', { name: 'bad' })).toBeNull();
    expect(screen.getByText(/\[bad\]\(javascript:alert\(1\)\)/)).toBeInTheDocument();
  });

  it('keeps snake_case words and lone asterisks as text', () => {
    const { container } = render(<Markdown source={'drop met_in_person and last_contact_before * 2'} />);
    expect(container.querySelector('em')).toBeNull();
    expect(container).toHaveTextContent('drop met_in_person and last_contact_before * 2');
  });
});
