// Every Phase 4 playbook runner against a real `openstance serve` on the demo
// dataset: the plan shows, the result renders human-first, and nothing is
// sent until the Spend button is pressed. Runs after fixes.spec.ts and before
// smoke.spec.ts (one graph, one worker). The last test imports a second
// LinkedIn export, so it changes the graph; it only moves one connection.
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { expect, test, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { api, cli, open } from './helpers';

test.describe.configure({ mode: 'serial' });

interface EnrichLog {
  totals: { requests_sent: number; spent_micro_usd: number };
}

async function nothingSent(request: APIRequestContext) {
  const log = await api<EnrichLog>(request, '/enrich-log');
  expect(log.totals.requests_sent).toBe(0);
  expect(log.totals.spent_micro_usd).toBe(0);
}

async function runner(page: Page, name: string): Promise<Locator> {
  await open(page, `playbooks/${name}`);
  const r = page.getByRole('region', { name: new RegExp(`run ${name}`, 'i') });
  await expect(r).toBeVisible();
  return r;
}

const result = (r: Locator) => r.getByRole('region', { name: /^result$/i });
const planTable = (r: Locator) => r.locator('table.plan-steps');
const spendButton = (r: Locator) => r.getByRole('button', { name: /^spend/i });

test('reconnect: local run shows the plan and a digest table, never a Spend button', async ({ page, request }) => {
  const r = await runner(page, 'reconnect');
  await r.getByRole('button', { name: /run playbook/i }).click();
  await expect(planTable(r)).toBeVisible();
  await expect(r.getByText(/requests sent: 0/i)).toBeVisible();
  const table = result(r).getByRole('table', { name: /reconnect digest/i });
  await expect(table).toBeVisible();
  const rows = table.locator('tbody tr');
  expect(await rows.count()).toBeGreaterThan(0);
  await expect(rows.first().getByRole('link')).toHaveAttribute('href', /#\/people\/\d+/);
  await expect(spendButton(r)).toHaveCount(0);
  await expect(result(r).locator('details.raw-json')).not.toHaveAttribute('open', '');
  await nothingSent(request);
});

test('accounts: coverage table uses the engine counts, not the capped lists', async ({ page, request }) => {
  const r = await runner(page, 'accounts');
  await r.getByRole('textbox', { name: /companies/i }).fill('Hexwire\nnorthwind.example');
  await r.getByRole('button', { name: /run playbook/i }).click();
  await expect(planTable(r)).toBeVisible();
  const rows = result(r).getByRole('table', { name: /account coverage/i }).locator('tbody tr');
  await expect(rows).toHaveCount(2);
  // the demo has far more than 10 people at Hexwire; the list is capped at 10
  const current = Number(await rows.nth(0).locator('td').nth(1).textContent());
  expect(current).toBeGreaterThan(10);
  await expect(rows.nth(0)).toContainText('warm');
  await expect(rows.nth(1)).toContainText(/northwind\.example.*none/);
  await expect(result(r).getByText(/showing 10 of \d+/).first()).toBeAttached();
  await expect(spendButton(r)).toHaveCount(0);
  await nothingSent(request);
});

test('verify: quote first; the address and the freshest mutual render; nothing sent', async ({ page, request }) => {
  const r = await runner(page, 'verify');
  await r.getByRole('textbox', { name: /^person/i }).fill('Maya Chen');
  await r.getByRole('button', { name: /get plan and quote/i }).click();
  await expect(planTable(r)).toBeVisible();
  await expect(planTable(r)).toContainText('skip_no_egress');
  await expect(r.getByRole('region', { name: /quote/i })).toContainText(/1 request/);
  await expect(spendButton(r)).toBeVisible();
  const card = result(r).locator('.verify-card');
  await expect(card.locator('code')).toHaveText(/@/);
  await expect(card).toContainText('unverified');
  await expect(card.getByText(/freshest mutual who replied/i)).toBeVisible();
  await expect(r.getByText(/requests sent: 0/i)).toBeVisible();
  await nothingSent(request);
});

test('refresh: a free quote still waits for Spend; after Spend the failures render', async ({ page, request }) => {
  const r = await runner(page, 'refresh');
  await r.getByRole('button', { name: /get plan and quote/i }).click();
  await expect(planTable(r)).toBeVisible();
  await expect(planTable(r)).toContainText('skip_no_egress');
  await expect(result(r).getByText(/no records updated/i)).toBeVisible();
  const spend = r.getByRole('button', { name: 'Spend $0.00' });
  await expect(spend).toBeVisible();
  await nothingSent(request);

  // treg cannot look up a profile with no LinkedIn or company: offline, and
  // with no network call, each line fails and says why
  await spend.click();
  await expect(r.getByText(/^Plan · /)).toContainText('partial');
  await expect(result(r).getByText(/^Failed:/)).toContainText(/needs a LinkedIn URL or handle, or a company/);
  await expect(spendButton(r)).toHaveCount(0);
  await nothingSent(request);
});

test('watch: one export explains itself; a second export shows the move into a target', async ({ page, request }) => {
  const r = await runner(page, 'watch');
  await r.getByRole('textbox', { name: /target companies/i }).fill('Carebridge');
  await r.getByRole('button', { name: /get plan and quote/i }).click();
  await expect(planTable(r)).toBeVisible();
  await expect(r.getByText(/needs two Connections\.csv exports/)).toBeVisible();
  await expect(result(r)).toContainText(/0 moves/);
  await expect(spendButton(r)).toHaveCount(0);

  // a newer export: one person moves from Hexwire to Carebridge
  const repo = resolve(import.meta.dirname, '../..');
  const lines = readFileSync(join(repo, 'fixtures/demo/Connections.csv'), 'utf8').split('\n');
  const i = lines.findIndex((l) => l.includes(',Hexwire,'));
  expect(i).toBeGreaterThan(0);
  const [first, last] = lines[i]!.split(',');
  lines[i] = lines[i]!.replace(/,Hexwire,[^,]*,/, ',Carebridge,Design Lead,');
  const dir = mkdtempSync(join(tmpdir(), 'openstance-e2e-li-'));
  writeFileSync(join(dir, 'Connections.csv'), lines.join('\n'));
  cli('import', join(dir, 'Connections.csv'));

  // same runner, same input: ask again
  await r.getByRole('button', { name: /get plan and quote/i }).click();
  const moves = result(r).getByRole('table', { name: /job moves/i }).locator('tbody tr');
  await expect(moves).toHaveCount(1);
  await expect(moves.first()).toContainText(`${first!.replace(/^\uFEFF/, '')} ${last}`);
  await expect(moves.first()).toContainText(/Hexwire.*Design Lead · Carebridge.*target/);
  await expect(result(r)).toContainText(/1 move into your target companies/);
  await expect(result(r).getByText(/^Comparing /)).toBeVisible();
  await nothingSent(request);
});
