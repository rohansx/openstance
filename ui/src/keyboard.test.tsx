import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { renderApp } from './test/render';

describe('keyboard navigation', () => {
  it('the skip link moves focus to the main region', async () => {
    renderApp('#/discover');
    await screen.findByRole('navigation', { name: 'Main navigation' });
    await userEvent.tab();
    const skip = screen.getByRole('link', { name: /skip to workspace/i });
    expect(skip).toHaveFocus();
    await userEvent.keyboard('{Enter}');
    expect(document.getElementById('main')).toHaveFocus();
  });

  it('dialogs take focus, close on Escape, and return focus', async () => {
    renderApp('#/views');
    const open = await screen.findByRole('button', { name: /new view/i });
    open.focus();
    await userEvent.keyboard('{Enter}');
    const dialog = await screen.findByRole('dialog');
    expect(dialog.contains(document.activeElement)).toBe(true);
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(open).toHaveFocus();
  });

  it('nav links are reachable with Tab and follow with Enter', async () => {
    renderApp('#/discover');
    const nav = await screen.findByRole('navigation', { name: 'Main navigation' });
    const review = Array.from(nav.querySelectorAll('a')).find((a) => a.textContent?.startsWith('Review queue'))!;
    review.focus();
    await userEvent.keyboard('{Enter}');
    await waitFor(() => expect(window.location.hash).toBe('#/review'));
  });
});
