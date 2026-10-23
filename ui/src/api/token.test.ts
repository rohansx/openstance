import { describe, expect, it, vi } from 'vitest';
import { TOKEN_KEY, clearToken, readToken, takeTokenFromLocation } from './token';

describe('token handoff (#token= fragment)', () => {
  it('reads the token from the fragment, keeps it in sessionStorage and strips it from the address bar', () => {
    window.history.replaceState(null, '', '/#token=Zk3abc_-DEF');
    const replace = vi.spyOn(window.history, 'replaceState');
    const push = vi.spyOn(window.history, 'pushState');

    expect(takeTokenFromLocation()).toBe('Zk3abc_-DEF');

    expect(sessionStorage.getItem(TOKEN_KEY)).toBe('Zk3abc_-DEF');
    expect(window.location.hash).not.toContain('token');
    expect(window.location.href).not.toContain('Zk3abc');
    // replaced in place: the token never lands in the back/forward history
    expect(replace).toHaveBeenCalled();
    expect(push).not.toHaveBeenCalled();
  });

  it('keeps a route that follows the token', () => {
    window.history.replaceState(null, '', '/#token=t0k&route=/review');
    expect(takeTokenFromLocation()).toBe('t0k');
    expect(window.location.hash).toBe('#/review');
  });

  it('never writes the token to localStorage', () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    window.history.replaceState(null, '', '/#token=secret1');
    takeTokenFromLocation();
    for (const call of setItem.mock.calls) {
      // every write went to sessionStorage, not localStorage
      expect(call[0]).toBe(TOKEN_KEY);
    }
    expect(localStorage.getItem(TOKEN_KEY)).toBeNull();
    expect(JSON.stringify({ ...localStorage })).not.toContain('secret1');
  });

  it('falls back to the token already in sessionStorage when there is no fragment', () => {
    sessionStorage.setItem(TOKEN_KEY, 'stored');
    window.history.replaceState(null, '', '/#/discover');
    expect(takeTokenFromLocation()).toBe('stored');
    expect(window.location.hash).toBe('#/discover');
  });

  it('returns null without a token, and clearToken forgets it', () => {
    window.history.replaceState(null, '', '/');
    expect(takeTokenFromLocation()).toBeNull();
    sessionStorage.setItem(TOKEN_KEY, 'x');
    clearToken();
    expect(readToken()).toBeNull();
  });
});
