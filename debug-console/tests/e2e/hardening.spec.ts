import { test, expect, type APIRequestContext, type Page } from '@playwright/test';

// Review-defect closure: backoff, stale, 410 banner, session/401, step-up, F11, F21, single live region.
const reset = (request: APIRequestContext, scenario = 'default') => request.post('/__fixture/reset', { data: { scenario } });
const fault = (request: APIRequestContext, data: object) => request.post('/__fixture/fault', { data });
const emit = (request: APIRequestContext, node: string, status: string, deliver = 'yes') =>
  request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: node, status, deliver } });
const state = (page: Page) => page.getByTestId('stream-status');

test.beforeEach(async ({ request }) => { await reset(request); });
test.afterEach(async ({ request }) => { await reset(request); });

test('backoff: the first reconnect attempt waits at least ~1 s (plan 16.13.4)', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(state(page)).toHaveAttribute('data-state', 'live');
  const times: number[] = [];
  page.on('request', (r) => { if (r.url().includes('/events/stream')) times.push(Date.now()); });
  const cutAt = Date.now();
  await request.post('/__fixture/cut');
  await expect(state(page)).toHaveAttribute('data-state', 'live', { timeout: 10000 });
  expect(times.length).toBeGreaterThan(0);
  expect(times[0]! - cutAt).toBeGreaterThanOrEqual(900);
});

test('stale: no events and no heartbeat for 2x the interval marks the view stale; a heartbeat clears it', async ({ page, request }) => {
  await page.route('**/config.json', (r) => r.fulfill({ json: { provider: 'fixture', sseHeartbeatMs: 200 } }));
  await request.post('/__fixture/heartbeat', { data: { ms: 200 } });
  await page.goto('/#/run/run-active');
  await expect(state(page)).toHaveAttribute('data-state', 'live');
  await page.waitForTimeout(1200); // heartbeats keep arriving: still live
  await expect(state(page)).toHaveAttribute('data-state', 'live');
  await request.post('/__fixture/heartbeat', { data: { muted: true } });
  await expect(state(page)).toHaveAttribute('data-state', 'stale', { timeout: 5000 });
  await expect(state(page)).toContainText('desactualizados');
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-status', 'planned'); // silence is not completion
  await request.post('/__fixture/heartbeat', { data: { muted: false } });
  await expect(state(page)).toHaveAttribute('data-state', 'live', { timeout: 5000 });
});

test('410: banner says earlier history was purged and names the recovery floor; no events are invented', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(state(page)).toHaveAttribute('data-state', 'live');
  await emit(request, 'verify', 'running');
  await emit(request, 'evaluate', 'running', 'none');
  await fault(request, { stream: 'gone' });
  await request.post('/__fixture/cut');
  const banner = page.getByTestId('history-purged');
  await expect(banner).toBeVisible({ timeout: 10000 });
  await expect(banner).toContainText('purgado');
  await expect(banner).toContainText('secuencia 2');
  await expect(page.getByTestId('node-evaluate')).toHaveAttribute('data-status', 'running'); // from the snapshot
  await expect(state(page)).toHaveAttribute('data-state', 'live');
});

test('session fetch failure is visible and mutations are never sent with an empty CSRF', async ({ page, request }) => {
  await fault(request, { session: 'down' });
  const posts: string[] = [];
  page.on('request', (r) => { if (r.method() === 'POST') posts.push(r.url()); });
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('session-banner')).toHaveAttribute('data-state', 'unavailable');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await expect(page.getByTestId('decision-phase')).toHaveAttribute('data-phase', 'failed');
  expect(posts).toEqual([]);
});

test('a non-SSE 401 mid-session shows session-expired', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(state(page)).toHaveAttribute('data-state', 'live');
  await expect(page.getByTestId('session-banner')).toHaveCount(0);
  await fault(request, { api: 'unauthorized' });
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await expect(page.getByTestId('session-banner')).toHaveAttribute('data-state', 'expired');
});

test('step-up failure is reported and nothing is approved', async ({ page, request }) => {
  await fault(request, { stepup: 'down' });
  await page.goto('/#/run/run-active');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await page.getByRole('button', { name: 'Reautenticar' }).click();
  await expect(page.getByTestId('decision-phase')).toHaveAttribute('data-phase', 'step_up_failed');
  expect(Object.keys(((await (await request.get('/__fixture/state')).json()) as { commands: object }).commands)).toEqual([]);
});

test('F21 stale expected_revision: conflict shown, no silent resubmit, reload then approve succeeds', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByRole('button', { name: 'Aprobar' })).toBeVisible();
  await request.post('/__fixture/bump_decision');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  const stale = page.getByTestId('decision-stale');
  await expect(stale).toContainText('revisión esperada 1');
  await expect(stale).toContainText('actual 2');
  await expect(page.getByRole('button', { name: 'Aprobar' })).toBeDisabled();
  expect(Object.keys(((await (await request.get('/__fixture/state')).json()) as { commands: object }).commands)).toEqual([]);
  await page.getByRole('button', { name: 'Recargar decisión' }).click();
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await page.getByRole('button', { name: 'Reautenticar' }).click();
  await expect(page.getByTestId('decision-phase')).toHaveAttribute('data-phase', 'succeeded');
});

test('F11 collector down: degraded trace panel, durable graph still shown; healthy run shows ok', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('trace-panel')).toHaveAttribute('data-state', 'ok');
  await reset(request, 'collector_down');
  await page.goto('/#/run/run-collector');
  const panel = page.getByTestId('trace-panel');
  await expect(panel).toHaveAttribute('data-state', 'degraded');
  await expect(panel).toContainText('sin trace_id');
  await expect(panel).toContainText('línea de tiempo durable');
  await expect(page.getByTestId('node-scout')).toBeVisible();
});

test('exactly one live region; a node change announces only that node, never the whole graph', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(state(page)).toHaveAttribute('data-state', 'live');
  expect(await page.locator('[aria-live],[role=status],[role=alert],[role=log]').count()).toBe(1);
  await emit(request, 'verify', 'running');
  const live = page.getByTestId('live-region');
  await expect(live).toContainText('Verify hypothesis: running');
  await expect(live).not.toContainText('Scout sources');
  await expect(live).not.toContainText('Evaluate proposal');
});

test('a burst of 30 node changes updates the live region at most a few times (rate limited)', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(state(page)).toHaveAttribute('data-state', 'live');
  await page.evaluate(() => {
    const el = document.querySelector('[data-testid=live-region]')!;
    (window as unknown as { __live: number }).__live = 0;
    new MutationObserver(() => { (window as unknown as { __live: number }).__live += 1; }).observe(el, { childList: true, characterData: true, subtree: true });
  });
  for (let i = 0; i < 30; i += 1) await emit(request, 'evaluate', i % 2 ? 'running' : 'queued');
  await page.waitForTimeout(500);
  expect(await page.evaluate(() => (window as unknown as { __live: number }).__live)).toBeLessThanOrEqual(3);
});
