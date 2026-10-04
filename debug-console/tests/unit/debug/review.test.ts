// @vitest-environment node
// Adversarial-review regressions (journal C-0002): gaps the first contract suite left vacuous.
import { describe, expect, it } from 'vitest';
import { CommandAck } from '../../../src/api/debug/dto';
import { SCOPE_FOR_COMMAND } from '../../../src/api/debug/commands';
import { createDebugApi } from '../../../src/api/debug';
import type { StreamHandlers } from '../../../src/api/debug/port';
import { makeHarness, noSleep } from './harness';

const until = async (cond: () => boolean, ms = 3000) => {
  const t0 = Date.now();
  while (!cond()) { if (Date.now() - t0 > ms) throw new Error('timeout waiting for condition'); await new Promise((r) => setTimeout(r, 5)); }
};
const handlers = (over: Partial<StreamHandlers> = {}) => {
  const got: number[] = []; const fatal: string[] = []; const resets: number[] = []; let drops = 0; let opens = 0;
  const h: StreamHandlers = { onEvent: (e) => got.push(e.sequence), onReset: (r) => resets.push(r.floor), onOpen: () => { opens += 1; }, onDrop: () => { drops += 1; }, onFatal: (r) => fatal.push(r), ...over };
  return { h, got, fatal, resets, drops: () => drops, opens: () => opens };
};
const enc = new TextEncoder();
const problem = (status: number, body: object) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

describe('stream: hostile or surprising inputs', () => {
  it('drops a foreign-run event even when its sequence is the next expected one', async () => {
    const hz = await makeHarness('fixture');
    const runId = (await hz.api.runs()).items[0]!.run_id;
    const last = (await hz.api.events(runId, 0)).items.at(-1)?.sequence ?? 0;
    const s = handlers(); const stop = hz.api.openStream(runId, s.h, { afterSequence: last });
    await until(() => s.opens() > 0);
    hz.backend.emitRaw(runId, { ...hz.backend.emit(runId), run_ref: 'other-run', sequence: last + 1 });
    await new Promise((r) => setTimeout(r, 40));
    expect(s.got).toEqual([last + 1]); // exactly the real one, once
    stop(); await hz.close();
  });

  it('a 410 whose snapshot ref points at another run is terminal and never resets the projection', async () => {
    const fetchImpl = async (url: string) => {
      if (url.includes('/stream')) return problem(410, { code: 'cursor_expired', recovery: { snapshot_ref: '/internal/v1/debug/runs/OTHER/graph', after_sequence: 5 } });
      throw new Error(`unexpected ${url}`);
    };
    const api = createDebugApi({ provider: 'http', fetch: fetchImpl, sleep: noSleep });
    const s = handlers(); api.openStream('mine', s.h, { afterSequence: 0 });
    await until(() => s.fatal.length > 0);
    expect(s.fatal).toEqual(['forbidden']); expect(s.resets).toEqual([]);
  });

  it('sends the cursor as after_sequence on every (re)connect, not only as Last-Event-ID', async () => {
    const hz = await makeHarness('fixture');
    const runId = (await hz.api.runs()).items[0]!.run_id;
    const last = (await hz.api.events(runId, 0)).items.at(-1)?.sequence ?? 0;
    const s = handlers(); const stop = hz.api.openStream(runId, s.h, { afterSequence: last });
    await until(() => s.opens() > 0);
    expect(hz.backend.lastResume()?.afterSequence).toBe(String(last));
    const e = hz.backend.emit(runId); await until(() => s.got.length === 1);
    hz.backend.closeStreams(); await until(() => s.opens() > 1);
    expect(hz.backend.lastResume()).toEqual({ afterSequence: String(e.sequence), lastEventId: String(e.sequence) });
    stop(); await hz.close();
  });

  it('401 on the stream is terminal (session_expired) and flips the session-expired callback', async () => {
    let expired = 0;
    const api = createDebugApi({ provider: 'http', fetch: async () => problem(401, { code: 'unauthenticated' }), sleep: noSleep, onSessionExpired: () => { expired += 1; } });
    const s = handlers(); api.openStream('r', s.h, { afterSequence: 0 });
    await until(() => s.fatal.length > 0);
    expect(s.fatal).toEqual(['session_expired']); expect(expired).toBe(1);
  });

  it('a frame that never terminates cannot grow the buffer without bound: the reader drops and reconnects', async () => {
    let calls = 0; let pulled = 0;
    const fetchImpl = async () => {
      calls += 1;
      const chunk = enc.encode(`data: ${'x'.repeat(256 * 1024)}`);
      return new Response(new ReadableStream<Uint8Array>({ async pull(c) { pulled += 1; await new Promise((r) => setTimeout(r, 2)); c.enqueue(chunk); } }), { status: 200 });
    };
    const api = createDebugApi({ provider: 'http', fetch: fetchImpl, sleep: noSleep });
    const s = handlers(); const stop = api.openStream('r', s.h, { afterSequence: 0 });
    await until(() => calls >= 2, 4000);
    stop();
    expect(s.got).toEqual([]); expect(s.drops()).toBeGreaterThan(0);
    expect(pulled).toBeLessThan(40); // < ~10 MiB read before giving up on the first connection
  }, 8000);

  it('stop() silences every handler, including a backfill that was in flight', async () => {
    const hz = await makeHarness('fixture');
    const runId = (await hz.api.runs()).items[0]!.run_id;
    const last = (await hz.api.events(runId, 0)).items.at(-1)?.sequence ?? 0;
    const s = handlers(); const stop = hz.api.openStream(runId, s.h, { afterSequence: last });
    await until(() => s.opens() > 0);
    stop();
    hz.backend.emit(runId); hz.backend.emit(runId);
    await new Promise((r) => setTimeout(r, 60));
    expect(s.got).toEqual([]);
    await hz.close();
  });
});

describe('commands: CSRF and ack semantics', () => {
  it('after a 401 the CSRF token is discarded and re-read from the session before the next mutation', async () => {
    const sessions: number[] = []; const csrfs: (string | null)[] = [];
    let n = 0;
    const fetchImpl = async (url: string, init?: RequestInit) => {
      if (url.endsWith('/api/v1/auth/session')) { n += 1; sessions.push(n); return new Response(JSON.stringify({ principal: 'p', tenant_id: 't', scopes: [], expires_at: 'x', csrf_token: `csrf-${n}`, auth: { simulated: true, level: 'basic', auth_at: 'x' } })); }
      csrfs.push(new Headers(init?.headers).get('x-csrf-token'));
      return csrfs.length === 1 ? problem(401, { code: 'unauthenticated' }) : new Response(JSON.stringify({ command_ref: { kind: 'command', id: 'c' }, status_url: '/s', state: 'requested' }), { status: 202 });
    };
    const api = createDebugApi({ provider: 'http', fetch: fetchImpl, sleep: noSleep });
    const c = { kind: 'pause' as const, runId: 'r', expectedRevision: 1, idempotencyKey: 'k', reason: 'why' };
    await expect(api.command(c)).rejects.toMatchObject({ status: 401 });
    await api.command(c);
    expect(csrfs).toEqual(['csrf-1', 'csrf-2']);
  });
  it('an ack is always "requested": a server claiming "confirmed" on the ack is never shown as confirmed', () => {
    const base = { command_ref: { kind: 'command', id: 'c' }, status_url: '/s' };
    expect(CommandAck.parse(base).state).toBe('requested');
    expect(CommandAck.parse({ ...base, state: 'confirmed' }).state).toBe('requested');
  });
  it('each command has its own, distinct, namespaced scope', () => {
    for (const [name, scope] of Object.entries(SCOPE_FOR_COMMAND)) expect(scope).toBe(`debug:command:${name}`);
  });
});
