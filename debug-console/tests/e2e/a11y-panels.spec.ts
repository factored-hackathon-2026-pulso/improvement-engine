import AxeBuilder from '@axe-core/playwright';
import { test, expect, type APIRequestContext, type Page } from '@playwright/test';

// axe coverage for S3 (opportunity/investigation), S4 (proposal diff), S5 (gates) and S6 (decision), in their
// meaningful states. "No axe critical/serious violations on the listed screens" - never "WCAG compliant".
const reset = (request: APIRequestContext, scenario = 'default') => request.post('/__fixture/reset', { data: { scenario } });
test.afterEach(async ({ request }) => { await reset(request); });

async function serious(page: Page, include?: string) {
  let b = new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'best-practice']);
  if (include) b = b.include(include);
  const r = await b.analyze();
  return r.violations.filter((v) => v.impact === 'critical' || v.impact === 'serious')
    .map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`);
}

const SECTIONS: [string, string, string, string][] = [
  ['S3 opportunity/investigation (refuted)', 'default', '/#/run/run-refuted', '[data-testid=investigation]'],
  ['S3 investigation (poison notes as text)', 'poison_notes', '/#/run/run-poison', '[data-testid=investigation]'],
  ['S4 proposal diff', 'default', '/#/run/run-refuted', '[data-testid=diff]'],
  ['S5 gates (insufficient_power / hold)', 'default', '/#/run/run-refuted', '[data-testid=gates]'],
  ['S5 gates (native fail, revise)', 'fail_revise', '/#/run/run-fail', '[data-testid=gates]'],
  ['S5 gates (unsafe + failed_infra)', 'failed_infra', '/#/run/run-infra', '[data-testid=gates]'],
  ['S6 decision (idle)', 'default', '/#/run/run-active', '[data-testid=decision]'],
  ['S6 decision (no permission)', 'bot_vs_human', '/#/run/run-active', '[data-testid=decision]'],
  ['trace panel (degraded)', 'collector_down', '/#/run/run-collector', '[data-testid=trace-panel]'],
];
for (const [name, scenario, url, include] of SECTIONS) {
  test(`axe: no critical/serious violations in ${name}`, async ({ page, request }) => {
    await reset(request, scenario);
    await page.goto(url);
    await page.locator(include).waitFor();
    await expect(page.locator(include)).toBeVisible();
    expect(await serious(page, include)).toEqual([]);
  });
}

test('axe: S6 decision in step-up-needed, step-up-failed and stale states', async ({ page, request }) => {
  await reset(request);
  await page.goto('/#/run/run-active');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await page.getByRole('button', { name: 'Reautenticar' }).waitFor();
  expect(await serious(page, '[data-testid=decision]')).toEqual([]);
  await request.post('/__fixture/fault', { data: { stepup: 'down' } });
  await page.getByRole('button', { name: 'Reautenticar' }).click();
  await expect(page.getByTestId('decision-phase')).toHaveAttribute('data-phase', 'step_up_failed');
  expect(await serious(page, '[data-testid=decision]')).toEqual([]);
  await reset(request);
  await page.goto('/#/run/run-active');
  await page.getByRole('button', { name: 'Aprobar' }).waitFor();
  await request.post('/__fixture/bump_decision');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await page.getByTestId('decision-stale').waitFor();
  expect(await serious(page, '[data-testid=decision]')).toEqual([]);
});

test('axe: full page in stale, purged-history and session-expired states', async ({ page, request }) => {
  await reset(request);
  await page.route('**/config.json', (r) => r.fulfill({ json: { provider: 'fixture', sseHeartbeatMs: 200 } }));
  await request.post('/__fixture/heartbeat', { data: { ms: 200 } });
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live');
  await request.post('/__fixture/heartbeat', { data: { muted: true } });
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'stale', { timeout: 5000 });
  expect(await serious(page)).toEqual([]);
  await request.post('/__fixture/heartbeat', { data: { muted: false } });
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live', { timeout: 5000 });
  await request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: 'verify', status: 'running' } });
  await request.post('/__fixture/fault', { data: { stream: 'gone' } });
  await request.post('/__fixture/cut');
  await page.getByTestId('history-purged').waitFor({ timeout: 10000 });
  expect(await serious(page)).toEqual([]);
  await request.post('/__fixture/fault', { data: { api: 'unauthorized' } });
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await page.getByTestId('session-banner').waitFor();
  expect(await serious(page)).toEqual([]);
});
