import { render } from '@testing-library/react';
import { TOKEN_KEY } from '../api/token';
import { Root } from '../Root';
import { defaultRoutes } from './fixtures';
import { mockApi } from './mockApi';

/** Render the whole app at `hash` with a token and the default fixture routes. */
export function renderApp(hash: string, routes: Record<string, unknown> = {}) {
  sessionStorage.setItem(TOKEN_KEY, 'test-token');
  window.history.replaceState(null, '', `/${hash}`);
  const api = mockApi({ ...defaultRoutes(), ...routes });
  const utils = render(<Root />);
  return { api, ...utils };
}
