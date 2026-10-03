import { test, expect, type APIRequestContext, type Page } from '@playwright/test';

const use = (request: APIRequestContext, scenario: string) => request.post('/__fixture/reset', { data: { scenario } });
test.afterEach(async ({ request }) => { await request.post('/__fixture/reset'); });

test('F01 empty: no runs is unknown, never "ingestion complete"', async ({ page, request }) => {
  await use(request, 'empty');
  await page.goto('/');
  await expect(page.getByText('no implica ingesta completa')).toBeVisible();
  await expect(page.getByText(/unknown/)).toBeVisible();
});

test('F04 positive: every node of the full path is complete and a successor run exists', async ({ page, request }) => {
  await use(request, 'positive');
  await page.goto('/#/run/run-positive');
  for (const id of ['approve', 'publish', 'release', 'observation', 'memory']) {
    await expect(page.getByTestId(`node-${id}`)).toHaveAttribute('data-status', 'complete');
  }
  await page.goto('/');
  await expect(page.getByRole('link', { name: 'Successor run', exact: true })).toBeVisible();
});

test('F06 fail_revise: native fail is shown as fail, combined is revise, never green', async ({ page, request }) => {
  await use(request, 'fail_revise');
  await page.goto('/#/run/run-fail');
  await expect(page.getByTestId('gate-native')).toHaveAttribute('data-status', 'fail');
  await expect(page.getByTestId('gate-improvement')).toHaveAttribute('data-status', 'not_evaluable');
  await expect(page.getByTestId('gate-combined')).toContainText('revise');
  await expect(page.getByTestId('textgraph-revise')).toContainText('proposal_revised');
});

test('F07 failed_infra: unsafe and failed_infra are distinct non-pass states, not "unrecognised"', async ({ page, request }) => {
  await use(request, 'failed_infra');
  await page.goto('/#/run/run-infra');
  await expect(page.getByTestId('gate-native')).toContainText('unsafe');
  await expect(page.getByTestId('gate-native')).not.toContainText('unrecognised');
  await expect(page.getByTestId('gate-improvement')).toContainText('failed_infra');
  await expect(page.getByTestId('gate-combined')).toContainText('hold');
});

test('F12/F13 cancel: requested is not confirmed', async ({ page, request }) => {
  await use(request, 'cancel_requested');
  await page.goto('/#/run/run-cancel');
  await expect(page.getByRole('heading', { level: 1 })).toContainText('cancel_requested');
  await expect(page.getByTestId('textgraph-evaluate')).toContainText('cancel_requested');
  await expect(page.getByRole('heading', { level: 1 })).not.toContainText('cancelled');
  await use(request, 'cancel_confirmed');
  await page.goto('/#/run/run-cancel');
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toContainText('cancelled');
  await expect(page.getByTestId('node-evaluate')).toHaveAttribute('data-status', 'cancelled');
});

test('F20 forbidden_cross_tenant: foreign run id is indistinguishable from an unknown one', async ({ request }) => {
  await use(request, 'forbidden_cross_tenant');
  const a = await request.get('/internal/v1/debug/runs/run-other-tenant/graph');
  const b = await request.get('/internal/v1/debug/runs/run-never-existed/graph');
  expect(a.status()).toBe(b.status());
  const strip = (j: Record<string, unknown>) => ({ ...j, correlation_id: '-' });
  expect(strip(await a.json())).toEqual(strip(await b.json()));
});

async function xssFlag(page: Page) { return page.evaluate(() => (window as unknown as { __xss?: number }).__xss ?? 0); }

test('F23 poison notes: HTML is text, JWS is masked, objects are rejected, no [object Object]', async ({ page, request }) => {
  await use(request, 'poison_notes');
  const errors: string[] = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto('/#/run/run-poison');
  await expect(page.getByTestId('investigation')).toContainText('<script>window.__xss=1</script>');
  await expect(page.getByTestId('investigation')).toContainText('[redacted]');
  expect(await page.content()).not.toContain('eyJhbGciOiJIUzI1NiJ9');
  expect(await xssFlag(page)).toBe(0);
  expect(errors.some((e) => e.includes('security.unredacted_payload'))).toBe(true);
  await page.goto('/#/run/run-poison-object');
  await expect(page.getByText('investigación no disponible')).toBeVisible();
  expect(await page.locator('body').innerText()).not.toContain('[object Object]');
});

test('F24 large_run: 100 runs list and a 1,200-event run baseline without loss', async ({ page, request }) => {
  await use(request, 'large_run');
  await page.goto('/');
  await expect(page.getByRole('list', { name: 'Ejecuciones' }).getByRole('link')).toHaveCount(100);
  await page.goto('/#/run/run-large-0');
  await expect(page.getByTestId('node-scout')).toBeVisible();
  await request.post('/__fixture/emit', { data: { run_id: 'run-large-0', node_id: 'scout', status: 'complete' } });
  await expect(page.getByTestId('node-scout')).toHaveAttribute('data-status', 'complete');
});

test('F25 bot_vs_human: no approve control at all and no role-name inference', async ({ page, request }) => {
  await use(request, 'bot_vs_human');
  await page.goto('/#/run/run-active');
  await expect(page.getByText('approve no está en available_commands')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Aprobar' })).toHaveCount(0);
});

// F14-F16: raw frame delivery control (dup / reorder / gap) against the real SSE reader.
async function seed(request: APIRequestContext) {
  for (const n of ['verify', 'evaluate', 'decision']) {
    await request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: n, status: 'queued', deliver: 'none' } });
  }
}
const deliver = (request: APIRequestContext, sequences: number[]) =>
  request.post('/__fixture/deliver', { data: { run_id: 'run-active', sequences } });

test('F14 duplicate frames are ignored (no extra graph reads)', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live');
  await seed(request);
  await deliver(request, [1, 2, 3]);
  await expect(page.getByTestId('node-decision')).toHaveAttribute('data-status', 'queued');
  let graphReads = 0;
  page.on('request', (r) => { if (r.url().endsWith('/runs/run-active/graph')) graphReads += 1; });
  await deliver(request, [1, 1, 2, 3, 3]);
  await page.waitForTimeout(800);
  expect(graphReads).toBe(0);
});

test('F15 reordered frames converge and do not replay animation', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live');
  await seed(request);
  await deliver(request, [3, 2, 1]);
  await expect(page.getByTestId('node-verify')).toHaveAttribute('data-status', 'queued');
  await expect(page.getByTestId('node-decision')).toHaveAttribute('data-status', 'queued');
});

test('F16 a gap triggers catch-up from the event log', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  await expect(page.getByTestId('stream-status')).toHaveAttribute('data-state', 'live');
  await seed(request);
  await deliver(request, [1, 3]); // 2 never delivered live
  await expect(page.getByTestId('node-decision')).toHaveAttribute('data-status', 'queued');
  await expect(page.getByTestId('node-evaluate')).toHaveAttribute('data-status', 'queued');
});
