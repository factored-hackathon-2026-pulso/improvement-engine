// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { DebugApiError, type RunEvent } from '../../../src/api/debug/dto';
import type { ProviderKind, StreamHandlers } from '../../../src/api/debug/port';
import { makeHarness, type Harness } from './harness';

const until = async (cond: () => boolean, ms = 3000) => {
  const t0 = Date.now();
  while (!cond()) {
    if (Date.now() - t0 > ms) throw new Error('timeout waiting for condition');
    await new Promise((r) => setTimeout(r, 5));
  }
};
const rejects = async (p: Promise<unknown>): Promise<DebugApiError> => {
  try { await p; } catch (e) { expect(e).toBeInstanceOf(DebugApiError); return e as DebugApiError; }
  throw new Error('expected rejection');
};

describe.each<ProviderKind>(['fixture', 'stand-in', 'http'])('DebugApi contract: %s provider', (kind) => {
  let h: Harness;
  let runId: string;
  const lastSeq = async () => (await h.api.events(runId, 0)).items.at(-1)?.sequence ?? 0;
  beforeEach(async () => { h = await makeHarness(kind); runId = (await h.api.runs()).items[0]!.run_id; });
  afterEach(async () => { await h.close(); });

  it('declares provider, target, runtime_profile and doubles', async () => {
    const m = await h.api.mode();
    expect(m.provider).toBe(kind);
    expect(typeof m.target).toBe('string');
    expect(typeof m.runtime_profile).toBe('string');
    expect(Array.isArray(m.doubles)).toBe(true);
    if (kind === 'stand-in') expect(m.doubles.length).toBeGreaterThan(0);
  });

  it('lists runs with the projection envelope, filters by state and clamps the page size', async () => {
    const l = await h.api.runs();
    expect(l.items.length).toBeGreaterThan(0);
    expect(l.schema_version).toBe('1');
    expect(Array.isArray(l.available_commands)).toBe(true);
    expect((await h.api.runs({ state: 'no_such_state' })).items).toEqual([]);
    expect((await h.api.runs({ limit: 5000 })).items.length).toBeGreaterThan(0); // clamped to 100, never a 4xx
  });

  it('reads graph, events, model-calls, queries, evals, memory-diff, external-commands and dependency health', async () => {
    expect((await h.api.graph(runId)).nodes.length).toBeGreaterThan(0);
    const ev = await h.api.events(runId, 0);
    const seqs = ev.items.map((e) => e.sequence);
    expect(seqs).toEqual([...seqs].sort((a, b) => a - b));
    if (seqs.length > 1) expect((await h.api.events(runId, seqs[0]!)).items.every((e) => e.sequence > seqs[0]!)).toBe(true);
    await h.api.modelCalls(runId);
    await h.api.queries(runId);
    for (const e of (await h.api.evals(runId)).items) { expect(e.core_gate).toBeDefined(); expect(e.improvement_gate).toBeDefined(); }
    await h.api.memoryDiff(runId);
    await h.api.externalCommands(runId);
    expect(Array.isArray((await h.api.dependencyHealth()).items)).toBe(true);
  });

  it('maps every failure to one error envelope (404 not retryable, 503 retryable) carrying a correlation id', async () => {
    const nf = await rejects(h.api.graph('no-such-run'));
    expect([nf.status, nf.code, nf.retryable]).toEqual([404, 'not_found', false]);
    expect(nf.correlationId.length).toBeGreaterThan(0);
    h.backend.failNext(503, 'dependency_unavailable');
    const down = await rejects(h.api.runs());
    expect([down.status, down.code, down.retryable]).toEqual([503, 'dependency_unavailable', true]);
  });

  it('a malformed success body is rejected as invalid_response, never rendered', async () => {
    h.backend.corruptNext();
    expect((await rejects(h.api.runs())).code).toBe('invalid_response');
  });

  describe('commands', () => {
    const cmd = (extra: object = {}) => ({ kind: 'pause' as const, runId, expectedRevision: 0, idempotencyKey: 'k-1', reason: 'investigating', ...extra });
    it('are refused by the server when not in available_commands', async () => {
      h.backend.setRunCommands(runId, []);
      const e = await rejects(h.api.command({ ...cmd(), expectedRevision: (await h.api.graph(runId)).projection_revision }));
      expect([e.status, e.code]).toEqual([409, 'command_unavailable']);
    });
    it('succeed as "requested" (never "confirmed") and are idempotent per key', async () => {
      h.backend.setRunCommands(runId, ['pause', 'resume', 'cancel', 'fork_replay']);
      const rev = (await h.api.graph(runId)).projection_revision;
      const a = await h.api.command({ ...cmd(), expectedRevision: rev });
      expect(a.state).toBe('requested');
      const b = await h.api.command({ ...cmd(), expectedRevision: rev });
      expect(b.command_ref).toEqual(a.command_ref);
    });
    it('a stale expected_revision is a 409 stale_revision with the conflict', async () => {
      h.backend.setRunCommands(runId, ['pause']);
      const e = await rejects(h.api.command(cmd({ expectedRevision: -1 })));
      expect([e.status, e.code]).toEqual([409, 'stale_revision']);
      expect(e.conflict?.expected_revision).toBe(-1);
    });
    it('fork-replay targets a run and retry targets a job', async () => {
      h.backend.setRunCommands(runId, ['fork_replay']);
      const rev = (await h.api.graph(runId)).projection_revision;
      expect((await h.api.command({ kind: 'fork_replay', runId, expectedRevision: rev, idempotencyKey: 'k-fork', reason: 'replay' })).state).toBe('requested');
      await rejects(h.api.command({ kind: 'retry', jobId: 'no-such-job', expectedRevision: 0, idempotencyKey: 'k-retry', reason: 'x' }));
    });
  });

  describe('SSE stream', () => {
    const open = (after: number, opts: Partial<StreamHandlers> = {}) => {
      const got: RunEvent[] = []; const resets: { floor: number; nodes: number }[] = []; const fatal: string[] = []; let opens = 0;
      const stop = h.api.openStream(runId, {
        onEvent: (e) => got.push(e), onReset: (s) => resets.push({ floor: s.floor, nodes: s.snapshot.nodes.length }),
        onOpen: () => { opens += 1; }, onDrop: () => undefined, onFatal: (r) => fatal.push(r), ...opts,
      }, { afterSequence: after });
      return { got, resets, fatal, stop, opens: () => opens };
    };
    it('delivers only events after the cursor, then live ones, in order', async () => {
      const s = open(await lastSeq());
      await until(() => s.opens() > 0);
      const e1 = h.backend.emit(runId); const e2 = h.backend.emit(runId);
      await until(() => s.got.length === 2);
      expect(s.got.map((e) => e.sequence)).toEqual([e1.sequence, e2.sequence]);
      s.stop();
    });
    it('deduplicates by (run, sequence)', async () => {
      const s = open(await lastSeq());
      await until(() => s.opens() > 0);
      const e = h.backend.emit(runId);
      h.backend.emitRaw(runId, e); h.backend.emitRaw(runId, e);
      const e2 = h.backend.emit(runId);
      await until(() => s.got.length >= 2);
      await new Promise((r) => setTimeout(r, 30));
      expect(s.got.map((x) => x.sequence)).toEqual([e.sequence, e2.sequence]);
      s.stop();
    });
    it('ignores events of another run', async () => {
      const s = open(await lastSeq());
      await until(() => s.opens() > 0);
      h.backend.emitRaw(runId, { ...h.backend.emit(runId), run_ref: 'other-run', sequence: 9999 });
      await until(() => s.got.length >= 1);
      await new Promise((r) => setTimeout(r, 30));
      expect(s.got.some((x) => x.run_ref === 'other-run')).toBe(false);
      s.stop();
    });
    it('on a gap backfills the missing events from /events and keeps order', async () => {
      const s = open(await lastSeq());
      await until(() => s.opens() > 0);
      const [e1, e2, e3] = h.backend.emitHidden(runId, 3); // appended to the log, not pushed to streams
      h.backend.emitRaw(runId, e3!);
      await until(() => s.got.length === 3);
      expect(s.got.map((x) => x.sequence)).toEqual([e1!.sequence, e2!.sequence, e3!.sequence]);
      s.stop();
    });
    it('resumes after a dropped connection with Last-Event-ID, without repeats or loss', async () => {
      const s = open(await lastSeq());
      await until(() => s.opens() > 0);
      const a = h.backend.emit(runId);
      await until(() => s.got.length === 1);
      h.backend.closeStreams();
      const during = h.backend.emitHidden(runId, 1)[0]!; // arrives while disconnected
      await until(() => s.opens() > 1);
      const b = h.backend.emit(runId);
      await until(() => s.got.length === 3);
      expect(s.got.map((x) => x.sequence)).toEqual([a.sequence, during.sequence, b.sequence]);
      expect(h.backend.lastResume()).toMatchObject({ lastEventId: String(a.sequence) });
      s.stop();
    });
    it('410 cursor_expired: replaces the projection with the snapshot, resumes from the floor, invents nothing', async () => {
      for (let i = 0; i < 4; i += 1) h.backend.emit(runId);
      const events = (await h.api.events(runId, 0)).items;
      const floor = events[2]!.sequence;
      h.backend.purge(runId, floor);
      const s = open(0);
      await until(() => s.resets.length === 1);
      expect(s.resets[0]!.floor).toBe(floor);
      expect(s.resets[0]!.nodes).toBeGreaterThan(0);
      await until(() => s.got.length === events.filter((e) => e.sequence > floor).length);
      expect(s.got.every((e) => e.sequence > floor)).toBe(true);
      expect(s.fatal).toEqual([]);
      s.stop();
    });
    it('a 404 on the stream is terminal (no retry loop)', async () => {
      const fatal: string[] = [];
      const stop = h.api.openStream('no-such-run', {
        onEvent: () => undefined, onReset: () => undefined, onOpen: () => undefined, onDrop: () => undefined, onFatal: (r) => fatal.push(r),
      }, { afterSequence: 0 });
      await until(() => fatal.length === 1);
      expect(fatal[0]).toBe('forbidden');
      stop();
    });
  });
});
