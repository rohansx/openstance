// Token handoff (interfaces.md section 3): `openstance serve` prints
// `http://127.0.0.1:7411/#token=<token>`. Fragments are never sent to a
// server, so the token does not reach access logs. The UI reads it once,
// keeps it in sessionStorage (never localStorage, never a cookie) and strips
// it from the address bar without adding a history entry.

export const TOKEN_KEY = 'openstance.token';

function storage(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

// Fallback when sessionStorage is blocked: the token lives in memory for this
// page load only.
let memory: string | null = null;

export function readToken(): string | null {
  try {
    const s = storage();
    if (s) return s.getItem(TOKEN_KEY);
  } catch {
    // fall through
  }
  return memory;
}

export function saveToken(token: string): void {
  memory = token;
  try {
    storage()?.setItem(TOKEN_KEY, token);
  } catch {
    // blocked: keep the in-memory copy
  }
}

export function clearToken(): void {
  memory = null;
  try {
    storage()?.removeItem(TOKEN_KEY);
  } catch {
    // ignore
  }
}

/**
 * Take `#token=...` from the current URL if present (optionally followed by
 * `&route=/path`), store it, and replace the URL with the route. Returns the
 * token in effect, or null.
 */
export function takeTokenFromLocation(): string | null {
  const hash = window.location.hash;
  if (!hash.startsWith('#token=')) return readToken();

  const params = new URLSearchParams(hash.slice(1));
  const token = params.get('token')?.trim() ?? '';
  const route = params.get('route');
  const nextHash = route && route.startsWith('/') ? `#${route}` : '#/';
  const { pathname, search } = window.location;
  window.history.replaceState(window.history.state, '', `${pathname}${search}${nextHash}`);

  if (!token) return readToken();
  saveToken(token);
  return token;
}
