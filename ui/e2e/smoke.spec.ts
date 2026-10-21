// P4-UI-06 smoke tests: the embedded UI against a real `openstance serve`
// on the fictional demo dataset. Tests share one graph and run in order.
import { expect, test, type Page } from '@playwright/test';
import { api, base, cli, open } from './helpers';

test.describe.configure({ mode: 'serial' });

interface ReviewQueue {
  flagged: { person: { id: number; display_name: string }; confidence: number }[];
}
interface EnrichLog {
  totals: { requests_sent: number; spent_micro_usd: number };
}

const card = (page: Page, name: string) =>
  page.locator('article').filter({ has: page.getByRole('heading', { name, exact: true }) });

test('search: parsed filters become chips and results carry evidence', async ({ page }) => {
  await open(page);
  await page.getByRole('searchbox', { name: /search your network/i }).fill('fintech founders');
  await page.getByRole('button', { name: /find people/i }).click();

  const chips = page.getByRole('group', { name: /parsed filters/i });
  await expect(chips.getByText('founder')).toBeVisible();
  await expect(chips.getByText('fintech')).toBeVisible();
  const maya = card(page, 'Maya Chen');
  await expect(maya).toBeVisible();
  await expect(maya.getByRole('meter', { name: /relationship score/i })).toBeVisible();
  await expect(page.getByText(/openstance search --title founder --industry fintech/)).toBeVisible();
});

test('person: open from search, edit the title, and it survives a rebuild', async ({ page }) => {
  await open(page);
  await page.getByRole('searchbox', { name: /search your network/i }).fill('fintech founders');
  await page.getByRole('button', { name: /find people/i }).click();
  await card(page, 'Maya Chen').getByRole('link', { name: /open maya chen/i }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Maya Chen' })).toBeVisible();
  const id = /#\/people\/(\d+)/.exec(page.url())![1]!;

  const title = page.getByRole('group', { name: 'Title' });
  await title.getByRole('button', { name: /edit title/i }).click();
  await title.getByRole('textbox', { name: /title/i }).fill('Chief Executive');
  await title.getByRole('button', { name: /^save$/i }).click();
  await expect(title.getByText('Chief Executive')).toBeVisible();
  await expect(title.getByText(/your edit/i)).toBeVisible();

  // Recompute everything derived from sources; the override must stay.
  cli('rebuild');
  const who = JSON.parse(cli('--json', 'who', id)) as { person: { title: string } };
  expect(who.person.title).toBe('Chief Executive');
  await page.reload();
  await expect(page.getByRole('heading', { level: 1, name: 'Maya Chen' })).toBeVisible();
  await expect(page.getByRole('group', { name: 'Title' }).getByText('Chief Executive')).toBeVisible();
});

test('review: confirming a flagged merge removes it from the queue', async ({ page, request }) => {
  const before = await api<ReviewQueue>(request, '/review');
  expect(before.flagged.length).toBeGreaterThan(0);
  const target = before.flagged[0]!.person;

  await open(page, 'review');
  const flagged = card(page, target.display_name);
  await expect(flagged).toBeVisible();
  await flagged.getByRole('button', { name: /^confirm merge/i }).click();
  await expect(flagged).toHaveCount(0);

  const after = await api<ReviewQueue>(request, '/review');
  expect(after.flagged.map((f) => f.person.id)).not.toContain(target.id);
  expect(after.flagged.length).toBe(before.flagged.length - 1);
});

test('path playbook: shows plan and quote; nothing is spent without the Spend button', async ({ page, request }) => {
  await open(page, 'playbooks/path');
  const runner = page.getByRole('region', { name: /run path/i });
  await runner.getByRole('textbox', { name: /^target/i }).fill('Noor Haddad');
  await runner.getByRole('textbox', { name: /^company/i }).fill('Nowhere Labs');
  await runner.getByRole('button', { name: /get plan and quote/i }).click();

  await expect(runner.getByText('lookup target in graph')).toBeVisible();
  const quote = runner.getByRole('region', { name: /quote/i });
  await expect(quote.getByText(/treg/)).toBeVisible();
  const spend = runner.getByRole('button', { name: /^Spend \$\d+\.\d+$/ });
  await expect(spend).toBeVisible();
  await expect(runner.getByText(/requests sent: 0/i)).toBeVisible();

  // The quote is recorded, but no request left the machine.
  const log = await api<EnrichLog>(request, '/enrich-log');
  expect(log.totals.requests_sent).toBe(0);
  expect(log.totals.spent_micro_usd).toBe(0);

  // A cap below the quote withdraws the button: nothing can be sent.
  await runner.getByRole('spinbutton', { name: /budget cap/i }).fill('0.5');
  await expect(spend).toHaveCount(0);
  await runner.getByRole('button', { name: /get plan and quote/i }).click();
  await expect(quote.getByText(/over your budget/i)).toBeVisible();
  await expect(spend).toHaveCount(0);
  expect((await api<EnrichLog>(request, '/enrich-log')).totals.requests_sent).toBe(0);
});

test('shortlist and introductions: save a person, track an intro, add one with a connector', async ({ page, request }) => {
  await open(page);
  await page.getByRole('searchbox', { name: /search your network/i }).fill('fintech founders');
  await page.getByRole('button', { name: /find people/i }).click();
  // (Maya Chen's title was edited above, so she no longer matches "founder")
  await expect(page.getByRole('group', { name: /parsed filters/i }).getByText('founder')).toBeVisible();
  const lucas = card(page, 'Lucas Moreau');
  await lucas.getByRole('button', { name: /save lucas moreau to shortlist/i }).click();
  await expect(lucas.getByRole('button', { name: /remove lucas moreau from shortlist/i })).toBeVisible();

  await page.goto(`${base()}/#/shortlist`);
  const row = page.locator('li, article').filter({ hasText: 'Lucas Moreau' }).first();
  await expect(row).toBeVisible();
  await row.getByRole('button', { name: /track an intro/i }).click();

  await page.goto(`${base()}/#/intros`);
  await expect(page.getByRole('region', { name: /^draft/i }).getByText('Lucas Moreau')).toBeVisible();

  await page.getByRole('button', { name: /new introduction/i }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox', { name: /who do you want to meet/i }).fill('Elif Kaya');
  await dialog.getByRole('button', { name: /suggest connectors/i }).click();
  const connector = dialog.getByRole('combobox', { name: /connector/i });
  await expect(connector.locator('option').nth(1)).toBeAttached();
  await connector.selectOption({ index: 1 });
  await dialog.getByRole('button', { name: /save to introductions/i }).click();
  await expect(page.getByRole('region', { name: /^draft/i }).getByText('Elif Kaya')).toBeVisible();

  const intros = await api<{ intros: { person: { display_name: string }; connector: { id: number } | null }[] }>(
    request,
    '/intros',
  );
  const elif = intros.intros.find((i) => i.person.display_name === 'Elif Kaya');
  expect(elif?.connector).not.toBeNull();
});

const ROUTES = ['discover', 'people', 'people/69', 'review', 'playbooks', 'playbooks/path', 'shortlist', 'intros', 'sources', 'views', 'egress', 'integrations', 'agents', 'settings'];

test('375 px: no page scrolls horizontally', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  await open(page);
  for (const route of ROUTES) {
    await page.goto(`${base()}/#/${route}`);
    await expect(page.locator('#main h1').first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, `#/${route} overflows by ${overflow}px`).toBeLessThanOrEqual(0);
  }
});

test('keyboard: skip link, then Tab to a nav link and follow it with Enter', async ({ page }) => {
  await open(page, 'discover');
  await page.keyboard.press('Tab');
  const skip = page.getByRole('link', { name: /skip to workspace/i });
  await expect(skip).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('#main')).toBeFocused();

  const nav = page.getByRole('navigation', { name: 'Main navigation' });
  const review = nav.getByRole('link', { name: /review queue/i });
  // From the top of a fresh page: skip link, then the sidebar links.
  await page.reload();
  await expect(nav).toBeVisible();
  for (let i = 0; i < 40 && !(await review.evaluate((el) => el === document.activeElement)); i++) {
    await page.keyboard.press('Tab');
  }
  await expect(review).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/#\/review$/);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
});

test('the API refuses requests without the token; /health does not need it', async ({ request }) => {
  expect((await request.get(`${base()}/health`)).ok()).toBe(true);
  expect((await request.get(`${base()}/stats`)).status()).toBe(401);
});
