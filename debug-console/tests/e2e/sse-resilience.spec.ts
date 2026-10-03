import { test, expect } from '@playwright/test';

// F14-F18 / F19: reconnect with backoff, resume without loss, 410 snapshot, 401 clean close.
test.beforeEach(async ({ request }) => { await request.post('/__fixture/reset'); });
const emit = (request: import('@playwright/test').APIRequestContext, node: string, status: string) =>
  request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: node, status } });

test('stream cut then reconnect resumes without losing events (F18)', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-status', 'planned');
  await request.post('/__fixture/cut');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'reconnecting');
  await emit(request, 'verify', 'running'); // happens while the stream is down
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live', { timeout: 10000 });
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-status', 'running');
  await emit(request, 'evaluate', 'running');
  await expect(page.getByTestId('node-evaluate')).toHaveAttribute('data-status', 'running');
});

test('410 purged cursor re-reads the snapshot and resubscribes (F17)', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live');
  await request.post('/__fixture/fault', { data: { stream: 'gone' } });
  await emit(request, 'verify', 'running'); // missed while the cursor is purged
  await request.post('/__fixture/cut');
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-status', 'running', { timeout: 10000 });
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live');
  await emit(request, 'evaluate', 'running');
  await expect(page.getByTestId('node-evaluate')).toHaveAttribute('data-status', 'running');
});

test('401 on reconnect closes cleanly without retry storm (F19)', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live');
  let hits = 0;
  page.on('request', (r) => { if (r.url().includes('/events/stream')) hits += 1; });
  await request.post('/__fixture/fault', { data: { stream: 'unauthorized' } });
  await request.post('/__fixture/cut');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'session_expired');
  await page.waitForTimeout(2500);
  expect(hits).toBe(1);
});

test('reconnect never steals focus', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  const trigger = page.getByTestId('node-hypothesis');
  await trigger.focus();
  await request.post('/__fixture/cut');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live', { timeout: 10000 });
  await expect(trigger).toBeFocused();
});
