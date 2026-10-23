import { execFileSync } from 'node:child_process';
import { expect, type APIRequestContext, type Page } from '@playwright/test';

export const base = () => process.env.E2E_BASE!;
export const token = () => process.env.E2E_TOKEN!;

/** Open the UI through the one-time #token= link, then go to `route`. */
export async function open(page: Page, route = '') {
  await page.goto(`${base()}/#token=${token()}`);
  await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible();
  // the token is moved to sessionStorage and removed from the address bar
  expect(page.url()).not.toContain(token());
  if (route) await page.goto(`${base()}/#/${route}`);
}

/** Call the API directly (bearer token), for assertions behind the UI. */
export async function api<T>(request: APIRequestContext, path: string): Promise<T> {
  const res = await request.get(`${base()}${path}`, { headers: { authorization: `Bearer ${token()}` } });
  expect(res.ok()).toBe(true);
  return (await res.json()) as T;
}

/** Run the CLI against the same home as the server. */
export function cli(...args: string[]): string {
  return execFileSync(process.env.E2E_BIN!, args, {
    env: {
      ...process.env,
      OPENSTANCE_HOME: process.env.E2E_HOME!,
      OPENSTANCE_OFFLINE: '1',
      OPENSTANCE__MODEL__EMBEDDER: 'none',
      OPENSTANCE__ENRICH__BACKEND: 'treg',
    // above the $0.76 profile estimate, so the Spend button can appear
    OPENSTANCE__ENRICH__MAX_USD_PER_RUN: '2',
    },
    encoding: 'utf8',
  });
}
