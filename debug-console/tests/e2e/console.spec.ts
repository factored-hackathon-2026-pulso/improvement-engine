import { test, expect } from '@playwright/test';

test.beforeEach(async ({ request }) => { await request.post('/__fixture/reset'); });

test('mode banner declares fixture profile and simulated identity', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByTestId('mode-banner')).toContainText('fixture');
  await expect(page.getByTestId('mode-banner')).toContainText('auth.simulated=true');
  await expect(page.getByRole('link', { name: 'Refuted hypothesis (kept visible)' })).toBeVisible();
});

test('drawer keeps instance, focus and scroll across 50+ SSE events; Escape returns focus', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  const trigger = page.getByTestId('node-hypothesis');
  await trigger.focus();
  await trigger.click();
  const drawer = page.getByTestId('drawer');
  await expect(drawer).toHaveAttribute('data-instance', 'hypothesis');
  await expect(drawer).toBeFocused();
  await page.evaluate(() => { document.querySelector('[data-testid=drawer]')!.setAttribute('data-marker', 'same'); });
  for (let i = 0; i < 52; i += 1) {
    await request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: 'evaluate', status: i % 2 ? 'running' : 'queued' } });
  }
  await expect(page.getByTestId('node-evaluate')).toHaveAttribute('data-status', 'running');
  await expect(drawer).toHaveAttribute('data-marker', 'same'); // never remounted
  await expect(drawer).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(drawer).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test('reduced motion disables pulses', async ({ page, request }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('node-verify')).toBeVisible();
  await request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: 'verify', status: 'running' } });
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-status', 'running');
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-pulse', '0');
});

test('live event pulses node without reload', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('node-verify')).toBeVisible();
  await request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: 'verify', status: 'running' } });
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-pulse', '1');
});

test('counter-evidence stays visible for a refuted run', async ({ page }) => {
  await page.goto('/#/run/run-refuted');
  await expect(page.getByTestId('verifier')).toHaveText('refuted');
  await expect(page.getByTestId('ev-contradicts')).toContainText('Holdout cohort shows no change');
  await expect(page.getByTestId('ev-limits')).toContainText('Only 2 of 13 sources');
  await expect(page.getByTestId('textgraph-verify')).toContainText('verifier_refuted');
});

test('two independent gates and a combined decision, never one green', async ({ page }) => {
  await page.goto('/#/run/run-refuted');
  await expect(page.getByTestId('gate-native')).toHaveAttribute('data-status', 'pass');
  await expect(page.getByTestId('gate-improvement')).toHaveAttribute('data-status', 'insufficient_power');
  await expect(page.getByTestId('gate-combined')).toContainText('hold');
});

test('unknown and blocked states are never shown as success', async ({ page }) => {
  await page.goto('/#/run/run-blocked');
  await expect(page.getByTestId('node-release')).toHaveAttribute('data-status', 'unknown');
  await expect(page.getByTestId('textgraph-release')).toContainText('release_ack_unknown');
  await expect(page.getByTestId('node-scout')).toHaveAttribute('data-status', 'waiting_dependency');
});

test('decision with step-up: requested is not confirmed until the receipt says so', async ({ page }) => {
  await page.goto('/#/run/run-active');
  await page.getByLabel('Nota').fill('looks fine');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await page.getByRole('button', { name: 'Reautenticar' }).click();
  const phase = page.getByTestId('decision-phase');
  await expect(phase).toHaveAttribute('data-phase', 'succeeded');
  await expect(phase).toContainText('Confirmada');
});

test('memory shows revoked item as forgotten', async ({ page }) => {
  await page.goto('/#/memory');
  await expect(page.getByTestId('mem-mem-2')).toContainText('Olvidado');
});
