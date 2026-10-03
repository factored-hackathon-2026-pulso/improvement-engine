import AxeBuilder from '@axe-core/playwright';
import { readFileSync } from 'node:fs';
import { test, expect, type APIRequestContext, type Page } from '@playwright/test';

// The demo (plan section 2) end to end in the console, from the demo driver world (loaded with /__fixture/load).
const world = JSON.parse(readFileSync('fixtures/demo-world.json', 'utf8'));
const load = async (request: APIRequestContext) => expect((await request.post('/__fixture/load', { data: world })).ok()).toBe(true);
test.beforeEach(async ({ request }) => { await load(request); });
test.afterEach(async ({ request }) => { await request.post('/__fixture/reset'); });

async function serious(page: Page, include?: string) {
  let b = new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'best-practice']);
  if (include) b = b.include(include);
  return (await b.analyze()).violations.filter((v) => v.impact === 'critical' || v.impact === 'serious')
    .map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`);
}

test('banner declares the stand-in demo: stand-in engine, scripted llm, fixture API, hook pending', async ({ page }) => {
  await page.goto('/');
  const b = page.getByTestId('mode-banner');
  await expect(b).toHaveAttribute('data-level', 'stand_in');
  await expect(b).toContainText('DEMO');
  await expect(b).toContainText('motor simulado (stand-in)');
  await expect(b).toContainText('LLM con guion');
  await expect(b).toContainText('API de fixture');
  await expect(b).toContainText('gancho pendiente');
  await page.getByText('Qué es simulado y hasta cuándo').click();
  await expect(page.getByTestId('doubles-detail')).toContainText('Rust engine HTTP API exists');
  await expect(page.getByRole('link', { name: /Undirected discovery run/ })).toBeVisible();
});

test('alternatives: do nothing versus the proposed change, with expected abandonment and risk', async ({ page }) => {
  await page.goto('/#/run/run-demo');
  await expect(page.getByTestId('alternatives')).toBeVisible();
  await expect(page.getByTestId('alt-alt-0')).toContainText('Hacer nada');
  await expect(page.getByTestId('alt-alt-0')).toContainText('1415');
  await expect(page.getByTestId('alt-alt-1')).toContainText('1029');
  await expect(page.getByTestId('alt-alt-1')).toContainText('guard exposure');
});

test('gate history: candidate 1 failed -> automatic revision -> candidate 2 passed', async ({ page }) => {
  await page.goto('/#/run/run-demo');
  await expect(page.getByTestId('attempt-1')).toHaveAttribute('data-outcome', 'fail');
  await expect(page.getByTestId('attempt-1')).toContainText('guard_breach');
  await expect(page.getByTestId('attempt-2')).toHaveAttribute('data-outcome', 'pass');
  await expect(page.getByTestId('attempt-2')).toContainText('revisión automática de cand-1');
  await expect(page.getByTestId('node-evaluate_1')).toHaveAttribute('data-status', 'dead');
  await expect(page.getByTestId('textgraph-evaluate_1')).toContainText('guard_breach');
  await expect(page.getByTestId('gate-native')).toHaveAttribute('data-status', 'pass');
  await expect(page.getByTestId('gate-combined')).toContainText('needs_human');
  await expect(page.getByTestId('native-report')).toContainText('95f9776f7db5');
});

test('investigation: several hypotheses, each with its verdict; the refuted decoy keeps its counterevidence', async ({ page }) => {
  await page.goto('/#/run/run-demo');
  await expect(page.getByTestId('hyp-main')).toHaveAttribute('data-verdict', 'supported');
  const decoy = page.getByTestId('hyp-card_replacement/otp_verify');
  await expect(decoy).toHaveAttribute('data-verdict', 'refuted');
  await expect(decoy).toContainText('4 of 7 weeks only');
  await expect(page.getByTestId('hyp-card_replacement/mobile')).toHaveAttribute('data-verdict', 'refuted');
  await page.goto('/#/run/run-demo-refuted');
  await expect(page.getByTestId('hyp-main')).toHaveAttribute('data-verdict', 'refuted');
  await expect(page.getByTestId('verifier')).toHaveText('refuted');
});

test('a run without evaluation never shows gates, attempts, alternatives or the diff of another run', async ({ page }) => {
  await page.goto('/#/run/run-demo-refuted');
  await expect(page.getByTestId('gate-native')).toHaveAttribute('data-status', 'not_evaluable');
  await expect(page.getByTestId('attempt-history')).toHaveCount(0);
  await expect(page.getByTestId('alternatives')).toHaveCount(0);
  await expect(page.getByTestId('diff')).toContainText('no produjo una propuesta');
  await page.goto('/#/run/run-demo');
  await expect(page.getByTestId('diff')).toContainText('max_retries: 3');
});

test('human-decision hook states are legible and the decision panel stays gated by step-up', async ({ page }) => {
  await page.goto('/#/run/run-demo');
  await expect(page.getByTestId('node-decision')).toHaveAttribute('data-hook', 'decision_pending');
  await expect(page.getByTestId('node-approve')).toHaveAttribute('data-hook', 'awaiting_authority');
  await expect(page.getByTestId('node-publish')).toHaveAttribute('data-hook', 'awaiting_authority');
  await expect(page.getByTestId('textgraph-decision')).toContainText('human_decision_pending');
  await expect(page.getByTestId('textgraph-decision')).toContainText('Gancho de decisión humana: pendiente');
  await expect(page.getByTestId('textgraph-approve')).toContainText('requiere autoridad humana');
  await expect(page.getByTestId('decision')).toContainText('Gancho de decisión humana pendiente');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await expect(page.getByText('Se requiere reautenticación humana.')).toBeVisible();
  await expect(page.getByTestId('decision-phase')).toHaveAttribute('data-phase', 'needs_step_up');
  await expect(page.getByTestId('node-decision')).toHaveAttribute('data-status', 'waiting_dependency'); // the console never advances it
});

test('the successor run is only planned', async ({ page }) => {
  await page.goto('/#/run/run-demo-successor');
  await expect(page.getByTestId('node-scout')).toHaveAttribute('data-status', 'planned');
  await expect(page.getByTestId('textgraph-scout')).toContainText('successor_not_started_by_stand_in');
});

for (const [name, url, sel] of [
  ['banner', '/', '[data-testid=mode-banner]'],
  ['alternatives', '/#/run/run-demo', '[data-testid=alternatives]'],
  ['gates with attempt history', '/#/run/run-demo', '[data-testid=gates]'],
  ['investigation with hypotheses', '/#/run/run-demo', '[data-testid=investigation]'],
  ['graph with hook states', '/#/run/run-demo', '.graph'],
] as const) {
  test(`axe: no critical/serious violations in demo ${name}`, async ({ page }) => {
    await page.goto(url);
    await page.locator(sel).waitFor();
    expect(await serious(page, sel)).toEqual([]);
  });
}
