// Regression flows from the manual Chrome pass (U1-U7), against a real
// `openstance serve` on the fictional demo dataset. Read-only: these tests do
// not change the graph.
import { expect, test, type Page } from '@playwright/test';
import { open } from './helpers';

const results = (page: Page) => page.getByRole('region', { name: /search results/i });
const resultsHeading = (page: Page) => results(page).getByRole('heading', { level: 2 }).first();

async function search(page: Page, text: string) {
  await page.getByRole('searchbox', { name: /search your network/i }).fill(text);
  await page.getByRole('button', { name: /find people/i }).click();
}

test('U1: a zero-result relax suggestion is a button whose rerun matches its count', async ({ page }) => {
  await open(page);
  await search(page, "fintech founders I've met");
  const relax = page.getByRole('button', { name: /drop “met in person” · \d+ results?/i });
  await expect(relax).toBeVisible();
  const promised = Number(/· (\d+) result/.exec((await relax.textContent()) ?? '')![1]);
  expect(promised).toBeGreaterThan(0);

  await relax.click();
  await expect(resultsHeading(page)).toContainText(`${promised} ${promised === 1 ? 'match' : 'matches'}`);
  await expect(page.getByRole('group', { name: /parsed filters/i }).getByText('met in person')).toHaveCount(0);
});

test('U2: strongest-relationships heading only for the empty query', async ({ page }) => {
  await open(page);
  await expect(resultsHeading(page)).toContainText('Your strongest relationships');
  await search(page, 'fintech founders');
  await expect(resultsHeading(page)).toContainText('Results for “fintech founders”');
  await expect(resultsHeading(page)).not.toContainText(/strongest/i);
});

test('U3: a route change starts at the top of the new page with focus on main', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 600 });
  await open(page);
  await search(page, 'fintech founders');
  await page.locator('article').first().getByRole('link', { name: /^open /i }).click();
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  const brief = page.getByRole('link', { name: /brief before you reach out/i });
  await brief.scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0);

  await brief.click();
  await expect(page.getByRole('region', { name: /run brief/i })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await expect(page.locator('#main')).toBeFocused();
});

test('U4: brief renders as markdown with Copy and Download, raw JSON collapsed', async ({ page }) => {
  await open(page);
  await search(page, 'fintech founders');
  await page.locator('article').first().getByRole('link', { name: /^open /i }).click();
  await page.getByRole('link', { name: /brief before you reach out/i }).click();
  const runner = page.getByRole('region', { name: /run brief/i });
  await runner.getByRole('button', { name: /get plan and quote/i }).click();
  const result = runner.getByRole('region', { name: /^result$/i });
  await expect(result.locator('.brief-md strong').first()).toBeVisible();
  await expect(result.locator('.brief-md li').first()).toBeVisible();
  await expect(result.getByRole('button', { name: /^copy$/i })).toBeVisible();
  await expect(result.getByRole('button', { name: /download \.md/i })).toBeVisible();
  await expect(result.locator('details.raw-json')).not.toHaveAttribute('open', '');
  await expect(result.locator('pre.result-json')).toBeHidden();
});

test('U4: path to someone in the graph renders ranked path cards with evidence', async ({ page }) => {
  await open(page);
  await search(page, 'fintech founders');
  const href = await page.locator('article').first().getByRole('link', { name: /^open /i }).getAttribute('href');
  const id = /#\/people\/(\d+)/.exec(href!)![1]!;
  await page.goto(`${page.url().split('#')[0]}#/playbooks/path`);
  const runner = page.getByRole('region', { name: /run path/i });
  await runner.getByRole('textbox', { name: /^target/i }).fill(id);
  await runner.getByRole('button', { name: /get plan and quote/i }).click();
  const result = runner.getByRole('region', { name: /^result$/i });
  await expect(result.getByText(/in your graph/i).first()).toBeVisible();
  const cards = result.getByRole('list', { name: 'Paths' }).getByRole('listitem', { name: /^path \d/i });
  await expect(cards.first()).toBeVisible();
  await expect(cards.first()).toContainText(/strength \d\.\d{3}/);
  await expect(cards.first().locator('.evidence-lines li').first()).toBeVisible();
});

test('U5: settings names the resolved graph.db from GET /config', async ({ page }) => {
  await open(page, 'settings');
  const db = page.locator('section.panel', { hasText: /on this device/i }).locator('code').first();
  await expect(db).toHaveText(`${process.env.E2E_HOME}/graph.db`);
});

test('U6: the import file input has an accessible label', async ({ page }) => {
  await open(page, 'sources');
  await expect(page.getByLabel('Choose export files')).toHaveAttribute('type', 'file');
});

test('U7: an unknown route shows not found with a link back to search', async ({ page }) => {
  await open(page, 'no-such-page');
  await expect(page.getByRole('heading', { name: /page not found/i })).toBeVisible();
  await page.getByRole('link', { name: /back to search/i }).click();
  await expect(page.getByRole('searchbox', { name: /search your network/i })).toBeVisible();
});
