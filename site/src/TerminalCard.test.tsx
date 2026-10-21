import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import TerminalCard from './TerminalCard';

describe('TerminalCard', () => {
  it('starts on the CLI tab with a real command', () => {
    render(<TerminalCard />);
    const cli = screen.getByRole('tab', { name: 'CLI' });
    expect(cli).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tabpanel')).toHaveTextContent('ost search "top 5 fintech founders who replied"');
  });

  it('switches to MCP and WEB on click', async () => {
    const user = userEvent.setup();
    render(<TerminalCard />);
    await user.click(screen.getByRole('tab', { name: 'MCP' }));
    expect(screen.getByRole('tab', { name: 'MCP' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'CLI' })).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByRole('tabpanel')).toHaveTextContent('search_people');

    await user.click(screen.getByRole('tab', { name: 'WEB' }));
    const panel = screen.getByRole('tabpanel');
    expect(panel).toHaveTextContent('openstance serve');
    expect(panel).toHaveTextContent('http://127.0.0.1:7411/#token=');
    expect(screen.getByText(/arriving in this release/i)).toBeInTheDocument();
  });

  it('supports arrow-key navigation with roving tabindex', async () => {
    const user = userEvent.setup();
    render(<TerminalCard />);
    const cli = screen.getByRole('tab', { name: 'CLI' });
    expect(cli).toHaveAttribute('tabindex', '0');
    expect(screen.getByRole('tab', { name: 'MCP' })).toHaveAttribute('tabindex', '-1');
    cli.focus();
    await user.keyboard('{ArrowRight}');
    expect(screen.getByRole('tab', { name: 'MCP' })).toHaveFocus();
    expect(screen.getByRole('tab', { name: 'MCP' })).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{ArrowLeft}{ArrowLeft}');
    expect(screen.getByRole('tab', { name: 'WEB' })).toHaveFocus();
    await user.keyboard('{Home}');
    expect(cli).toHaveFocus();
    await user.keyboard('{End}');
    expect(screen.getByRole('tab', { name: 'WEB' })).toHaveAttribute('aria-selected', 'true');
  });

  it('links each tab to its panel', () => {
    render(<TerminalCard />);
    const tab = screen.getByRole('tab', { name: 'CLI' });
    const panel = screen.getByRole('tabpanel');
    expect(tab.getAttribute('aria-controls')).toBe(panel.id);
    expect(panel.getAttribute('aria-labelledby')).toBe(tab.id);
  });
});
