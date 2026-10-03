import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { z } from 'zod';
import * as S from '../../src/api/schemas';
import { BASE, DEBUG, fixtureOnly, getJson, startTarget, stopTarget } from './helpers';

beforeAll(startTarget, 30000);
afterAll(stopTarget);

const ok = async <T>(schema: z.ZodType<T>, path: string) => {
  const r = await getJson(path);
  expect(r.status, path).toBe(200);
  const p = schema.safeParse(r.body);
  expect(p.success, `${path}: ${p.success ? '' : JSON.stringify(p.error.issues)}`).toBe(true);
  return r.body as T;
};

describe('every response validates against the zod schemas', () => {
  it('session and profile (consumer_proposal)', async () => {
    await ok(S.Session, '/api/v1/auth/session');
    await ok(S.Profile, `${DEBUG}/profile`);
  });
  it('run list, graph, events, investigation, gates for every run', async () => {
    const list = await ok(S.RunList, `${DEBUG}/runs`);
    expect(list.items.length).toBeGreaterThan(0);
    for (const r of list.items) {
      await ok(S.Graph, `${DEBUG}/runs/${r.run_id}/graph`);
      await ok(z.object({ items: z.array(S.DebugEventSchema) }), `${DEBUG}/runs/${r.run_id}/events?after_sequence=0`);
      await ok(S.Investigation, `${DEBUG}/runs/${r.run_id}/investigation`);
      await ok(S.Gates, `${DEBUG}/runs/${r.run_id}/gates`);
    }
  });
  it('diff, memory, decision (consumer_proposal)', async () => {
    await ok(S.Diff, `${DEBUG}/proposals/prop-1/diff`);
    await ok(S.Memory, `${DEBUG}/memory`);
    await ok(S.Decision, `${DEBUG}/decisions/dec-1`);
  });
  it('a missing projection_revision is rejected, never read as 0', () => {
    const body = { schema_version: '1', tenant_id: 't', status: 's', blocking_reasons: [], available_commands: [], nodes: [] };
    expect(S.Graph.safeParse(body).success).toBe(false);
  });
});

describe('Problem codes', () => {
  it('unknown run is a 404 Problem', async () => {
    const r = await getJson(`${DEBUG}/runs/nope/graph`);
    expect(r.status).toBe(404);
    expect(S.Problem.safeParse(r.body).success).toBe(true);
  });
});

describe('transport', () => {
  const body = JSON.stringify({ expected_revision: 1, response: 'approve', note: 'n' });
  const post = (headers: Record<string, string>, b = body) =>
    getJson(`${DEBUG}/decisions/dec-1/responses`, { method: 'POST', body: b, headers: { 'content-type': 'application/json', ...headers } });
  const csrf = async () => ((await getJson('/api/v1/auth/session')).body as { csrf_token: string }).csrf_token;

  it('POST without CSRF -> 403 csrf_failed', async () => {
    const r = await post({ 'idempotency-key': 'k-1' });
    expect(r.status).toBe(403);
    expect(S.Problem.parse(r.body).code).toBe('csrf_failed');
  });
  it('POST without Idempotency-Key -> 422', async () => {
    const r = await post({ 'x-csrf-token': await csrf() });
    expect(r.status).toBe(422);
  });
  it('non-string note -> 422 with field_errors, never coerced', async () => {
    const r = await post({ 'x-csrf-token': await csrf(), 'idempotency-key': 'k-2' }, JSON.stringify({ expected_revision: 1, response: 'approve', note: { a: 1 } }));
    expect(r.status).toBe(422);
    expect(JSON.stringify(r.body)).toContain('note');
  });
  it('GET has no effects', async () => {
    if (!fixtureOnly) return;
    const before = JSON.stringify((await getJson('/__fixture/state')).body);
    await getJson(`${DEBUG}/runs`); await getJson(`${DEBUG}/memory`); await getJson(`${DEBUG}/decisions/dec-1`);
    expect(JSON.stringify((await getJson('/__fixture/state')).body)).toBe(before);
  });
  it('SSE id equals sequence and Last-Event-ID resumes without loss or duplication', async () => {
    if (!fixtureOnly) return;
    await getJson('/__fixture/reset', { method: 'POST' });
    for (const s of ['running', 'queued', 'running']) {
      await getJson('/__fixture/emit', { method: 'POST', body: JSON.stringify({ run_id: 'run-active', node_id: 'verify', status: s }) });
    }
    const ctl = new AbortController();
    const res = await fetch(`${BASE}${DEBUG}/runs/run-active/events/stream`, { headers: { 'last-event-id': '1' }, signal: ctl.signal });
    const reader = res.body!.pipeThrough(new TextDecoderStream()).getReader();
    let text = '';
    while (!text.includes('id: 3')) text += (await reader.read()).value ?? '';
    ctl.abort();
    const ids = [...text.matchAll(/^id: (\d+)$/gm)].map((m) => Number(m[1]));
    expect(ids).toEqual([2, 3]);
    for (const m of text.matchAll(/^data: (.*)$/gm)) {
      const ev = S.DebugEventSchema.parse(JSON.parse(m[1]!));
      expect(ids).toContain(ev.sequence);
    }
  });
  it('repeated Idempotency-Key returns the same command; a different body under the same key is 409', async () => {
    if (!fixtureOnly) return;
    const token = await csrf();
    const h = { 'x-csrf-token': token, 'idempotency-key': 'k-same' };
    await getJson('/api/v1/auth/step-up', { method: 'POST', body: '{}', headers: { 'x-csrf-token': token, 'content-type': 'application/json' } });
    const a = await post(h);
    const b = await post(h);
    expect(a.status).toBe(202);
    expect(S.Accepted.parse(b.body).command_ref).toEqual(S.Accepted.parse(a.body).command_ref);
    const c = await post(h, JSON.stringify({ expected_revision: 1, response: 'approve', note: 'different' }));
    expect(c.status).toBe(409);
    expect(S.Problem.parse(c.body).code).toBe('idempotency_conflict');
  });
});

describe('local session cookie profile (plan 16.13.6 / A06)', () => {
  const cookies = (r: Response) => (r.headers as Headers & { getSetCookie(): string[] }).getSetCookie();
  it('sets exactly one HTTP-profile cookie: pulso_local_session, HttpOnly, SameSite=Lax, Path=/, no Domain, no Secure, not __Host-', async () => {
    if (!fixtureOnly) return;
    const res = await fetch(`${BASE}/api/v1/auth/session`);
    const set = cookies(res);
    expect(set).toHaveLength(1);
    const c = set[0]!;
    expect(c).toMatch(/^pulso_local_session=[A-Za-z0-9_-]{16,}/);
    expect(c).not.toMatch(/^__Host-/);
    expect(c).toMatch(/;\s*HttpOnly/i);
    expect(c).toMatch(/;\s*SameSite=Lax/i);
    expect(c).toMatch(/;\s*Path=\//i);
    expect(c).not.toMatch(/;\s*Domain=/i);
    expect(c).not.toMatch(/;\s*Secure/i); // loopback HTTP profile only; the deployed profile uses __Host- + Secure
  });
  it('the cookie value is opaque (not JWT-shaped), never echoed in the body, and not the CSRF token', async () => {
    if (!fixtureOnly) return;
    const res = await fetch(`${BASE}/api/v1/auth/session`);
    const value = /^pulso_local_session=([^;]+)/.exec(cookies(res)[0]!)![1]!;
    expect(value).not.toMatch(/^eyJ/);
    const body = (await res.json()) as { csrf_token: string };
    expect(JSON.stringify(body)).not.toContain(value);
    expect(body.csrf_token).not.toBe(value);
  });
  it('no other response sets a cookie', async () => {
    if (!fixtureOnly) return;
    for (const p of [`${DEBUG}/runs`, `${DEBUG}/memory`, `${DEBUG}/profile`]) expect(cookies(await fetch(`${BASE}${p}`)), p).toHaveLength(0);
  });
});

describe('decision revision CAS (F21), trace ids, 410 body, heartbeat', () => {
  const csrf = async () => ((await getJson('/api/v1/auth/session')).body as { csrf_token: string }).csrf_token;
  const post = async (b: unknown, key: string) => getJson(`${DEBUG}/decisions/dec-1/responses`, {
    method: 'POST', body: JSON.stringify(b), headers: { 'content-type': 'application/json', 'x-csrf-token': await csrf(), 'idempotency-key': key },
  });
  const stepUp = async () => getJson('/api/v1/auth/step-up', { method: 'POST', body: '{}', headers: { 'x-csrf-token': await csrf(), 'content-type': 'application/json' } });
  it('the decision projection carries a domain_revision', async () => {
    const d = S.Decision.parse((await getJson(`${DEBUG}/decisions/dec-1`)).body);
    expect(d.domain_revision).toBeGreaterThanOrEqual(1);
  });
  it('a missing expected_revision is a 422 with field_errors', async () => {
    if (!fixtureOnly) return;
    await getJson('/__fixture/reset', { method: 'POST' });
    await stepUp();
    const r = await post({ response: 'approve', note: 'n' }, 'k-norev');
    expect(r.status).toBe(422);
    expect(JSON.stringify(r.body)).toContain('expected_revision');
  });
  it('a stale expected_revision is 409 stale_revision with conflict{expected,current,diff_ref}; nothing is stored', async () => {
    if (!fixtureOnly) return;
    await getJson('/__fixture/reset', { method: 'POST' });
    await stepUp();
    await getJson('/__fixture/bump_decision', { method: 'POST' });
    const r = await post({ expected_revision: 1, response: 'approve', note: 'n' }, 'k-stale');
    expect(r.status).toBe(409);
    const p = S.Problem.parse(r.body);
    expect(p.code).toBe('stale_revision');
    expect(p.conflict).toEqual({ expected_revision: 1, current_revision: 2, diff_ref: null });
    expect(JSON.stringify((await getJson('/__fixture/state')).body)).not.toContain('cmd-k-stale');
  });
  it('graph nodes carry trace_id (32 lowercase hex or null)', async () => {
    const g = (await getJson(`${DEBUG}/runs/run-active/graph`)).body as { nodes: { trace_id: string | null }[] };
    for (const n of g.nodes) expect(n.trace_id === null || /^[0-9a-f]{32}$/.test(n.trace_id)).toBe(true);
  });
  it('410 is a cursor_expired Problem with recovery_after_sequence and snapshot_url', async () => {
    if (!fixtureOnly) return;
    await getJson('/__fixture/reset', { method: 'POST' });
    await getJson('/__fixture/emit', { method: 'POST', body: JSON.stringify({ run_id: 'run-active', node_id: 'verify', status: 'running' }) });
    await getJson('/__fixture/fault', { method: 'POST', body: JSON.stringify({ stream: 'gone' }) });
    const r = await getJson(`${DEBUG}/runs/run-active/events/stream`);
    expect(r.status).toBe(410);
    const b = r.body as { code: string; recovery_after_sequence: number; snapshot_url: string };
    expect(S.Problem.safeParse(b).success).toBe(true);
    expect(b.code).toBe('cursor_expired');
    expect(b.recovery_after_sequence).toBe(1);
    expect(b.snapshot_url).toBe(`${DEBUG}/runs/run-active/graph`);
  });
  it('SSE heartbeats are comment frames at the configured cadence', async () => {
    if (!fixtureOnly) return;
    await getJson('/__fixture/reset', { method: 'POST' });
    await getJson('/__fixture/heartbeat', { method: 'POST', body: JSON.stringify({ ms: 100 }) });
    const ctl = new AbortController();
    const res = await fetch(`${BASE}${DEBUG}/runs/run-active/events/stream`, { signal: ctl.signal });
    const reader = res.body!.pipeThrough(new TextDecoderStream()).getReader();
    let text = '';
    while ((text.match(/: hb/g) ?? []).length < 2) text += (await reader.read()).value ?? '';
    ctl.abort();
    await getJson('/__fixture/reset', { method: 'POST' });
    expect(text).toContain(': hb');
  });
});
