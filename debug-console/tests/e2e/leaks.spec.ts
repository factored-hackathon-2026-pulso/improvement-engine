import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test, expect, type Page } from '@playwright/test';

// Canary tests: planted CANARY_* secrets, the CSRF token and the session cookie must never reach URLs,
// request headers/bodies other than the intended ones, browser storage, document.cookie or the DOM.
const BASE = process.env.BASE_URL ?? 'http://127.0.0.1:5173';
const CSRF = 'fixture-csrf-token';
const CANARY = /CANARY_(?:SECRET|PII|FINAL)_[0-9a-f-]{8,}/i;

test.beforeEach(async ({ request }) => { await request.post('/__fixture/reset', { data: { scenario: 'canary' } }); });
test.afterEach(async ({ request }) => { await request.post('/__fixture/reset'); });

async function storageDump(page: Page) {
  return page.evaluate(async () => {
    const idb = 'databases' in indexedDB ? await indexedDB.databases() : [];
    return JSON.stringify({
      local: { ...localStorage }, session: { ...sessionStorage }, cookie: document.cookie, idb: idb.map((d) => d.name),
    });
  });
}

test('session cookie: exactly one pulso_local_session, HttpOnly, Lax, Path=/, not Secure, invisible to scripts', async ({ page, context }) => {
  await page.goto('/#/run/run-canary');
  await expect(page.getByTestId('investigation')).toBeVisible();
  const cookies = await context.cookies();
  expect(cookies.map((c) => c.name)).toEqual(['pulso_local_session']);
  const c = cookies[0]!;
  expect(c).toMatchObject({ httpOnly: true, sameSite: 'Lax', path: '/', secure: false });
  expect(c.name.startsWith('__Host-')).toBe(false);
  expect(await page.evaluate(() => document.cookie)).toBe('');
});

test('canary / CSRF / cookie never reach browser storage, document.cookie, the DOM or any URL (HAR)', async ({ browser }, info) => {
  const harPath = join(info.outputPath(), 'canary.har');
  const context = await browser.newContext({ baseURL: BASE, recordHar: { path: harPath, content: 'omit' } });
  const page = await context.newPage();
  const consoleErrors: string[] = [];
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  await page.goto('/#/run/run-canary');
  await expect(page.getByTestId('investigation')).toContainText('[redacted]');
  await page.getByLabel('Nota').fill('ok');
  await page.getByRole('button', { name: 'Aprobar' }).click();
  await page.getByRole('button', { name: 'Reautenticar' }).click();
  await expect(page.getByTestId('decision-phase')).toHaveAttribute('data-phase', 'succeeded');

  const cookieValue = (await context.cookies())[0]!.value;
  const dump = await storageDump(page);
  const dom = await page.content();
  for (const [name, text] of [['storage', dump], ['dom', dom]] as const) {
    expect(text, `${name} canary`).not.toMatch(CANARY);
    expect(text, `${name} csrf`).not.toContain(CSRF);
    expect(text, `${name} cookie`).not.toContain(cookieValue);
  }
  expect(consoleErrors.join('\n')).not.toMatch(CANARY);
  expect(consoleErrors.join('\n')).toContain('security.unredacted_payload');

  await context.close(); // flushes the HAR
  const har = JSON.parse(readFileSync(harPath, 'utf8')) as { log: { entries: { request: { url: string; method: string; headers: { name: string; value: string }[]; postData?: { text?: string }; cookies: { value: string }[] } }[] } };
  expect(har.log.entries.length).toBeGreaterThan(5);
  for (const { request: r } of har.log.entries) {
    expect(r.url, r.url).not.toMatch(CANARY);
    expect(r.url, r.url).not.toContain(CSRF);
    expect(r.url, r.url).not.toContain(cookieValue);
    expect(r.url, r.url).not.toMatch(/eyJ[A-Za-z0-9_-]{5,}\./);
    expect(r.headers.map((h) => h.name.toLowerCase()), r.url).not.toContain('authorization');
    expect(r.postData?.text ?? '', r.url).not.toContain(CSRF);
    // the CSRF token travels only in its own header, and only on mutations
    const withCsrf = r.headers.some((h) => h.name.toLowerCase() === 'x-csrf-token' && h.value === CSRF);
    if (withCsrf) expect(r.method).toBe('POST');
    // the session cookie only rides in the cookie header, never in another header
    for (const h of r.headers) if (h.name.toLowerCase() !== 'cookie') expect(h.value, `${r.url} ${h.name}`).not.toContain(cookieValue);
  }
});
