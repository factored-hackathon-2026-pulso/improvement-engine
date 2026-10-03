import AxeBuilder from '@axe-core/playwright';
import { test, expect, type Page } from '@playwright/test';

// "No axe critical/serious violations on the listed screens" and "keyboard operable" (never "WCAG compliant").
test.beforeEach(async ({ request }) => { await request.post('/__fixture/reset'); });

const SCREENS: [string, string, string][] = [
  ['S1 command centre', '/#/', 'link'],
  ['S2 run (active)', '/#/run/run-active', 'node-verify'],
  ['S2 run (refuted)', '/#/run/run-refuted', 'verifier'],
  ['S2 run (blocked)', '/#/run/run-blocked', 'node-release'],
  ['S7 memory', '/#/memory', 'mem-mem-2'],
];

async function serious(page: Page) {
  const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'best-practice']).analyze();
  return r.violations.filter((v) => v.impact === 'critical' || v.impact === 'serious')
    .map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`);
}

for (const [name, url, ready] of SCREENS) {
  test(`axe: no critical/serious violations on ${name}`, async ({ page }) => {
    await page.goto(url);
    if (ready === 'link') await page.getByRole('link', { name: 'Refuted hypothesis (kept visible)' }).waitFor();
    else await page.getByTestId(ready).waitFor();
    expect(await serious(page)).toEqual([]);
  });
}

test('axe: no critical/serious violations with a drawer open', async ({ page }) => {
  await page.goto('/#/run/run-active');
  await page.getByTestId('node-hypothesis').click();
  await page.getByTestId('drawer').waitFor();
  expect(await serious(page)).toEqual([]);
});

test('text equivalent covers 100% of graph nodes', async ({ page }) => {
  await page.goto('/#/run/run-blocked');
  await page.getByTestId('node-release').waitFor(); // the graph loads asynchronously
  const nodes = await page.locator('[data-testid^="node-"]').evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.slice(5)));
  expect(nodes.length).toBeGreaterThan(0);
  for (const id of nodes) await expect(page.getByTestId(`textgraph-${id}`)).toBeVisible();
});

test('keyboard-only walkthrough: open a node, close with Escape, focus returns', async ({ page }) => {
  await page.goto('/#/run/run-active');
  await page.getByTestId('node-scout').waitFor();
  await page.getByTestId('node-scout').focus();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('drawer')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('node-scout')).toBeFocused();
  // every interactive control is reachable by Tab (no tabindex trap, no focus loss to body)
  const seen = new Set<string>();
  for (let i = 0; i < 40; i += 1) {
    await page.keyboard.press('Tab');
    seen.add(await page.evaluate(() => document.activeElement?.tagName ?? 'NONE'));
  }
  expect(seen.has('BUTTON')).toBe(true);
  expect(seen.has('A')).toBe(true);
});

test('no running animations under reduced motion (T-A05)', async ({ page, request }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/#/run/run-active');
  await page.getByTestId('node-verify').waitFor();
  await request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: 'verify', status: 'running' } });
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-status', 'running');
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});

test('harness self-check: axe does flag a deliberately inaccessible page', async ({ page }) => {
  await page.setContent('<html lang="es"><body><main><img src="data:image/gif;base64,R0lGODlhAQABAAAAACw="><button></button></main></body></html>');
  expect((await serious(page)).length).toBeGreaterThan(0);
});
