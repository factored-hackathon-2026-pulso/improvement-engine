import { test, expect } from '@playwright/test';

// First RED (L7): the run graph must update from an SSE event, with no reload and no polling.
test.beforeEach(async ({ request }) => { await request.post('/__fixture/reset'); });

test('graph updates from an SSE event', async ({ page, request }) => {
  await page.goto('/#/run/run-active');
  const node = page.getByTestId('node-verify');
  await expect(node).toHaveAttribute('data-status', 'planned');
  await request.post('/__fixture/emit', { data: { run_id: 'run-active', node_id: 'verify', status: 'running' } });
  await expect(node).toHaveAttribute('data-status', 'running');
  await expect(page.getByTestId('textgraph-verify')).toContainText('running');
});
