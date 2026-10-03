// In-process implementation of the spec 25 debug routes over a `World`. It serves the `fixture` and `stand-in`
// providers (through the same http provider code path) and, wrapped in a node http server, the http-provider tests.
// It is a test double / demo model, not the control-api: it invents no endpoint beyond spec 25.
import type { z } from 'zod';
import type * as D from './dto';
import { SCOPE_FOR_COMMAND } from './commands';
import { DEBUG_PREFIX } from './http';

export interface RunRecord {
  summary: { run_id: string; title: string; state: string; origin: string };
  revision: number;
  nodes: z.input<typeof D.GraphNode>[];
  events: z.input<typeof D.RunEvent>[];
  modelCalls: z.input<typeof D.ModelCall>[];
  queries: z.input<typeof D.QueryReceipt>[];
  evals: z.input<typeof D.EvalCard>[];
  memoryDiff: Omit<z.input<typeof D.MemoryDiff>, keyof ReturnType<typeof envelopeOf>>;
  externalCommands: z.input<typeof D.ExternalCommand>[];
  jobs: string[];
  availableCommands: string[];
}
export interface World {
  tenant: string;
  mode: { target: string; runtime_profile: string; doubles: string[] };
  principal: string;
  /** Session scopes; command scopes are granted explicitly so tests can also remove them. */
  scopes: string[];
  runs: RunRecord[];
  health: z.input<typeof D.DependencyHealth>[];
}

const envelopeOf = (w: World, revision: number, commands: string[]) => ({
  schema_version: '1' as const, tenant_id: w.tenant, projection_revision: revision, as_of: '2026-01-01T00:00:00Z', status: 'ok', blocking_reasons: [] as string[], available_commands: commands,
});
export const ALL_COMMAND_SCOPES = Object.values(SCOPE_FOR_COMMAND);

export interface Backend {
  handle: (req: Request) => Promise<Response>;
  /** `fetch`-compatible function for in-process providers. */
  fetch: (url: string, init?: RequestInit) => Promise<Response>;
  /** Appends the next event (next sequence) to the run's log and pushes it to open streams. */
  emit: (runId: string) => z.input<typeof D.RunEvent>;
  /** Appends n events to the log WITHOUT pushing them (a connection gap). */
  emitHidden: (runId: string, n: number) => z.input<typeof D.RunEvent>[];
  /** Pushes an event to open streams without touching the log (duplicates, gaps, foreign runs). */
  emitRaw: (runId: string, e: z.input<typeof D.RunEvent>) => void;
  /** Purges the log through `seq`: older cursors now get 410 cursor_expired. */
  purge: (runId: string, seq: number) => void;
  closeStreams: () => void;
  setRunCommands: (runId: string, cmds: string[]) => void;
  failNext: (status: number, code: string) => void;
  corruptNext: () => void;
  lastResume: () => { afterSequence: string | null; lastEventId: string | null } | null;
}

export function createBackend(world: World): Backend {
  const runs = new Map(world.runs.map((r) => [r.summary.run_id, structuredClone(r)]));
  const floors = new Map<string, number>();
  const streams = new Set<{ runId: string; push: (e: z.input<typeof D.RunEvent>) => void; close: () => void }>();
  const acks = new Map<string, unknown>();
  let fail: { status: number; code: string } | null = null;
  let corrupt = false;
  let resume: { afterSequence: string | null; lastEventId: string | null } | null = null;
  let nextCmd = 1;
  const enc = new TextEncoder();
  const CSRF = 'backend-csrf-token';

  const json = (status: number, body: unknown, headers: Record<string, string> = {}) => {
    const b = status === 200 && corrupt ? { items: 'corrupt' } : body;
    if (status === 200) corrupt = false;
    return new Response(JSON.stringify(b), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store', ...headers } });
  };
  const problem = (status: number, code: string, extra: object = {}) =>
    json(status, { code, message: code, correlation_id: `corr-${Math.random().toString(16).slice(2, 10)}`, retryable: status === 503, ...extra });
  const page = (rec: RunRecord, items: unknown[]) => ({ ...envelopeOf(world, rec.revision, rec.availableCommands), items, next_cursor: null });
  const lastSeq = (rec: RunRecord) => rec.events.reduce((m, e) => Math.max(m, e.sequence), floors.get(rec.summary.run_id) ?? 0);
  const mk = (rec: RunRecord): z.input<typeof D.RunEvent> => ({
    sequence: lastSeq(rec) + 1, run_ref: rec.summary.run_id, stage: 'scan', event_code: 'job_heartbeat', status: 'ok', occurred_at: '2026-01-01T00:00:00Z',
  });
  const frame = (e: z.input<typeof D.RunEvent>) => enc.encode(`id: ${e.sequence}\ndata: ${JSON.stringify(e)}\n\n`);

  function stream(req: Request, rec: RunRecord, url: URL): Response {
    const runId = rec.summary.run_id;
    const lastEventId = req.headers.get('last-event-id');
    const afterParam = url.searchParams.get('after_sequence');
    resume = { afterSequence: afterParam, lastEventId };
    const after = Number(lastEventId ?? afterParam ?? 0); // Last-Event-ID wins over the query parameter
    const floor = floors.get(runId) ?? 0;
    if (!Number.isInteger(after) || after < 0) return problem(422, 'invalid_cursor');
    if (after < floor) {
      return problem(410, 'cursor_expired', { recovery: { snapshot_ref: `${DEBUG_PREFIX}/runs/${runId}/graph`, after_sequence: floor } });
    }
    let entry: { runId: string; push: (e: z.input<typeof D.RunEvent>) => void; close: () => void } | null = null;
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        let open = true;
        const close = () => { if (open) { open = false; try { c.close(); } catch { /* already closed */ } } };
        entry = { runId, push: (e) => { if (open) c.enqueue(frame(e)); }, close };
        for (const e of rec.events) if (e.sequence > after) c.enqueue(frame(e));
        streams.add(entry);
        req.signal.addEventListener('abort', () => { if (entry) streams.delete(entry); close(); }, { once: true });
      },
      cancel() { if (entry) streams.delete(entry); },
    });
    return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-store' } });
  }

  async function command(req: Request, runId: string | null, jobId: string | null, kind: string): Promise<Response> {
    if (req.headers.get('x-csrf-token') !== CSRF) return problem(403, 'csrf_invalid');
    const key = req.headers.get('idempotency-key');
    if (!key) return problem(422, 'idempotency_key_required');
    const body = (await req.json().catch(() => null)) as { expected_revision?: unknown; reason?: unknown } | null;
    if (!body || typeof body.reason !== 'string' || body.reason.length === 0 || typeof body.expected_revision !== 'number') return problem(422, 'invalid_command');
    const rec = runId ? runs.get(runId) : [...runs.values()].find((r) => r.jobs.includes(jobId ?? ''));
    if (!rec) return problem(404, 'not_found');
    const replay = acks.get(`${rec.summary.run_id}:${kind}:${key}`);
    if (replay) return json(200, replay);
    if (!rec.availableCommands.includes(kind)) return problem(409, 'command_unavailable');
    if (body.expected_revision !== rec.revision) {
      return problem(409, 'stale_revision', { conflict: { expected_revision: body.expected_revision, current_revision: rec.revision, diff_ref: null } });
    }
    const id = `cmd-${nextCmd++}`;
    const ack = { command_ref: { kind: 'command', id }, status_url: `${DEBUG_PREFIX}/commands/${id}`, state: 'requested' };
    acks.set(`${rec.summary.run_id}:${kind}:${key}`, ack);
    return json(202, ack);
  }

  async function handle(req: Request): Promise<Response> {
    const url = new URL(req.url);
    const p = url.pathname;
    if (fail) { const f = fail; fail = null; return problem(f.status, f.code); }
    if (p === '/api/v1/auth/session' && req.method === 'GET') {
      return json(200, { principal: world.principal, tenant_id: world.tenant, scopes: world.scopes, expires_at: '2099-01-01T00:00:00Z', csrf_token: CSRF, auth: { simulated: true, level: 'basic', auth_at: '2026-01-01T00:00:00Z' } });
    }
    if (!p.startsWith(`${DEBUG_PREFIX}/`)) return problem(404, 'not_found');
    const rest = p.slice(DEBUG_PREFIX.length + 1);
    if (req.method === 'POST') {
      const m = /^runs\/([^/]+)\/(pause|resume|cancel|fork-replay)$/.exec(rest);
      if (m) return command(req, decodeURIComponent(m[1]!), null, m[2]!.replace('-', '_'));
      const j = /^jobs\/([^/]+)\/retry$/.exec(rest);
      if (j) return command(req, null, decodeURIComponent(j[1]!), 'retry');
      return problem(404, 'not_found');
    }
    if (req.method !== 'GET') return problem(405, 'method_not_allowed');
    if (rest === 'profile') return json(200, world.mode);
    if (rest === 'health/dependencies') return json(200, { ...envelopeOf(world, 1, []), items: world.health, next_cursor: null });
    if (rest === 'runs') {
      const state = url.searchParams.get('state'); const trigger = url.searchParams.get('trigger');
      const limit = Number(url.searchParams.get('limit') ?? 100);
      if (!Number.isInteger(limit) || limit < 1 || limit > 100) return problem(422, 'invalid_limit');
      const items = [...runs.values()].filter((r) => (!state || r.summary.state === state) && (!trigger || r.summary.origin === trigger)).slice(0, limit)
        .map((r) => ({ ...r.summary, projection_revision: r.revision }));
      return json(200, { ...envelopeOf(world, 1, []), items, next_cursor: null });
    }
    const m = /^runs\/([^/]+)\/(graph|events|model-calls|queries|evals|memory-diff|external-commands|stream)$/.exec(rest);
    const rec = m ? runs.get(decodeURIComponent(m[1]!)) : undefined;
    if (!m || !rec) return problem(404, 'not_found');
    switch (m[2]) {
      case 'graph': return json(200, { ...envelopeOf(world, rec.revision, rec.availableCommands), nodes: rec.nodes });
      case 'events': {
        const after = Number(url.searchParams.get('after_sequence') ?? 0);
        if (!Number.isInteger(after) || after < 0) return problem(422, 'invalid_cursor');
        return json(200, page(rec, rec.events.filter((e) => e.sequence > after).sort((a, b) => a.sequence - b.sequence)));
      }
      case 'model-calls': return json(200, page(rec, rec.modelCalls));
      case 'queries': return json(200, page(rec, rec.queries));
      case 'evals': return json(200, page(rec, rec.evals));
      case 'memory-diff': return json(200, { ...envelopeOf(world, rec.revision, rec.availableCommands), ...rec.memoryDiff });
      case 'external-commands': return json(200, page(rec, rec.externalCommands));
      default: return stream(req, rec, url);
    }
  }

  const push = (runId: string, e: z.input<typeof D.RunEvent>) => streams.forEach((s) => { if (s.runId === runId) s.push(e); });
  return {
    handle,
    fetch: (url, init) => handle(new Request(new URL(url, 'http://backend.local'), init)),
    emit(runId) {
      const rec = runs.get(runId)!;
      const e = mk(rec); rec.events.push(e); rec.revision += 1; push(runId, e); return e;
    },
    emitHidden(runId, n) {
      const rec = runs.get(runId)!;
      return Array.from({ length: n }, () => { const e = mk(rec); rec.events.push(e); rec.revision += 1; return e; });
    },
    emitRaw: push,
    purge(runId, seq) {
      const rec = runs.get(runId)!;
      floors.set(runId, seq);
      rec.events = rec.events.filter((e) => e.sequence > seq);
    },
    closeStreams: () => { streams.forEach((s) => s.close()); streams.clear(); },
    setRunCommands: (runId, cmds) => { runs.get(runId)!.availableCommands = cmds; },
    failNext: (status, code) => { fail = { status, code }; },
    corruptNext: () => { corrupt = true; },
    lastResume: () => resume,
  };
}
