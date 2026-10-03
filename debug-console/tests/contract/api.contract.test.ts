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
  const body = JSON.stringify({ response: 'approve', note: 'n' });
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
    const r = await post({ 'x-csrf-token': await csrf(), 'idempotency-key': 'k-2' }, JSON.stringify({ response: 'approve', note: { a: 1 } }));
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
    const c = await post(h, JSON.stringify({ response: 'approve', note: 'different' }));
    expect(c.status).toBe(409);
    expect(S.Problem.parse(c.body).code).toBe('idempotency_conflict');
  });
});
