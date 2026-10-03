import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { makeWorld } from '../fixtures/world.mjs';

let world = makeWorld();
let faults = { stream: null }; // 'gone' (410 once) | 'unauthorized' (401 until reset)
const sseClients = new Set();
const CSRF = 'fixture-csrf-token';
const PORT = Number(process.env.FIXTURE_PORT ?? 4010);
const PREFIX = '/internal/v1/debug';

const env = (entity_ref, rev, status, extra = {}) => ({
  schema_version: '1', tenant_id: 'tenant-fixture', entity_ref, projection_revision: rev, as_of: '2026-01-01T00:00:00Z',
  environment: 'sandbox', source_kind: 'sandbox_evaluated', validation: 'unverified', status, blocking_reasons: [],
  next_automatic_action: null, available_commands: [], links: [],
  coverage: { status: 'complete', observed_count: null, expected_count: null, reason_code: null }, ...extra,
});
const send = (res, code, body) => {
  res.writeHead(code, { 'content-type': 'application/json', 'cache-control': 'no-store' });
  res.end(JSON.stringify(body));
};
const problem = (res, code, status, extra = {}) => send(res, status, {
  code, message: code, correlation_id: randomUUID(), retryable: false, current_ref: null, blocking_refs: [], conflict: null, ...extra,
});
const readBody = (req) => new Promise((ok) => {
  let d = '';
  req.on('data', (c) => { d += c; });
  req.on('end', () => { try { ok(d ? JSON.parse(d) : {}); } catch { ok(null); } });
});

function emit(runId, nodeId, status) {
  const run = world.runs[runId];
  if (!run) return null;
  const n = run.nodes.find((x) => x.node_id === nodeId);
  if (!n) return null;
  n.status = status;
  run.revision += 1;
  const list = world.events[runId];
  const ev = {
    event_id: randomUUID(), run_id: runId, sequence: list.length + 1,
    entity_ref: { kind: 'node', id: nodeId }, projection_revision: run.revision, kind: 'node_status_changed',
  };
  list.push(ev);
  for (const c of sseClients) if (c.runId === runId) c.res.write(`id: ${ev.sequence}\ndata: ${JSON.stringify(ev)}\n\n`);
  return ev;
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://x');
  const p = url.pathname;
  const m = req.method;
  if (p === '/healthz') return send(res, 200, { ok: true });
  if (p.startsWith('/__fixture/')) {
    if (p === '/__fixture/reset') {
      for (const c of sseClients) c.res.end();
      sseClients.clear();
      world = makeWorld();
      faults = { stream: null };
      return send(res, 200, { ok: true });
    }
    if (p === '/__fixture/emit') {
      const b = await readBody(req);
      const ev = b && emit(b.run_id, b.node_id, b.status);
      return ev ? send(res, 200, ev) : send(res, 400, { code: 'bad_emit' });
    }
    if (p === '/__fixture/fault') {
      const b = await readBody(req);
      if (!b || !['gone', 'unauthorized', null].includes(b.stream ?? null)) return send(res, 400, { code: 'bad_fault' });
      faults.stream = b.stream ?? null;
      return send(res, 200, { ok: true });
    }
    if (p === '/__fixture/cut') { // simulate a dropped transport / API restart: end every open stream
      for (const c of sseClients) c.res.end();
      sseClients.clear();
      return send(res, 200, { ok: true });
    }
    if (p === '/__fixture/state') return send(res, 200, world);
    return send(res, 404, { code: 'not_found' });
  }
  if (p === '/api/v1/auth/session') {
    return send(res, 200, {
      principal: 'fixture-human', tenant_id: 'tenant-fixture', scopes: ['debug/read', 'evolution/decide'],
      expires_at: '2099-01-01T00:00:00Z', csrf_token: CSRF,
      auth: { simulated: true, level: world.decision.stepped_up ? 'step_up' : 'basic', auth_at: '2026-01-01T00:00:00Z' },
    });
  }
  if (p === '/api/v1/auth/step-up' && m === 'POST') {
    if (req.headers['x-csrf-token'] !== CSRF) return problem(res, 'csrf_failed', 403);
    world.decision.stepped_up = true;
    return send(res, 200, { level: 'step_up' });
  }
  if (p === `${PREFIX}/profile`) {
    return send(res, 200, { target: 'mock', runtime_profile: 'fixture', doubles: ['api_fixture', 'sse_fixture', 'identity_fixture'], pin: null });
  }
  if (p === `${PREFIX}/runs`) {
    const items = Object.values(world.runs).map((r) => ({
      run_id: r.run_id, title: r.title, state: r.state, origin: r.origin, projection_revision: r.revision,
    }));
    return send(res, 200, { ...env(null, 1, 'ok'), items, next_cursor: null });
  }
  let mm = p.match(new RegExp(`^${PREFIX}/runs/([^/]+)/(graph|events|events/stream|investigation|gates)$`));
  if (mm) {
    const run = world.runs[mm[1]];
    if (!run) return problem(res, 'not_found', 404);
    const kind = mm[2];
    const ref = { kind: 'run', id: run.run_id };
    if (kind === 'graph') return send(res, 200, { ...env(ref, run.revision, run.state), nodes: run.nodes });
    if (kind === 'events') {
      const after = Number(url.searchParams.get('after_sequence') ?? 0);
      return send(res, 200, { items: world.events[run.run_id].filter((e) => e.sequence > after), next_cursor: null });
    }
    if (kind === 'investigation') {
      const inv = world.investigation[run.run_id] ?? { hypothesis: null, verifier: 'unknown', evidence: [] };
      return send(res, 200, { ...env(ref, run.revision, 'ok'), ...inv });
    }
    if (kind === 'gates') return send(res, 200, { ...env(ref, run.revision, 'ok'), ...world.gates });
    if (faults.stream === 'unauthorized') return problem(res, 'session_expired', 401);
    if (faults.stream === 'gone') { faults.stream = null; return problem(res, 'cursor_purged', 410); }
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' });
    res.write(': open\n\n');
    const resumeFrom = Number(req.headers['last-event-id'] ?? 0);
    if (resumeFrom > 0) {
      for (const e of world.events[run.run_id]) if (e.sequence > resumeFrom) res.write(`id: ${e.sequence}\ndata: ${JSON.stringify(e)}\n\n`);
    }
    const c ={ res, runId: run.run_id };
    sseClients.add(c);
    const hb = setInterval(() => res.write(': hb\n\n'), 5000);
    req.on('close', () => { clearInterval(hb); sseClients.delete(c); });
    return undefined;
  }
  if (p === `${PREFIX}/proposals/prop-1/diff`) {
    return send(res, 200, { ...env({ kind: 'proposal', id: 'prop-1' }, 1, 'ok'), ...world.diff });
  }
  if (p === `${PREFIX}/memory`) return send(res, 200, { ...env(null, 1, 'ok'), items: world.memory });
  if (p === `${PREFIX}/decisions/dec-1`) {
    return send(res, 200, {
      ...env({ kind: 'decision', id: 'dec-1' }, 1, 'pending', { available_commands: world.decision.available_commands }),
      needs_step_up: world.decision.needs_step_up && !world.decision.stepped_up,
    });
  }
  if (p === `${PREFIX}/decisions/dec-1/responses` && m === 'POST') {
    if (req.headers['x-csrf-token'] !== CSRF) return problem(res, 'csrf_failed', 403);
    const key = req.headers['idempotency-key'];
    if (!key) return problem(res, 'validation_error', 422);
    const b = await readBody(req);
    if (!b || typeof b.note !== 'string') {
      return problem(res, 'validation_error', 422, { field_errors: [{ field: 'note', code: 'must_be_string' }] });
    }
    if (world.decision.needs_step_up && !world.decision.stepped_up) return problem(res, 'waiting_human_reauthentication', 403);
    const id = `cmd-${key}`;
    world.commands[id] = { polls: 0 };
    return send(res, 202, {
      command_ref: { kind: 'external_command', id }, status_url: `${PREFIX}/commands/external_command/${id}`,
      entity_ref: { kind: 'decision', id: 'dec-1' },
    });
  }
  mm = p.match(new RegExp(`^${PREFIX}/commands/external_command/([^/]+)$`));
  if (mm) {
    const c = world.commands[mm[1]];
    if (!c) return problem(res, 'not_found', 404);
    c.polls += 1;
    return send(res, 200, { state: c.polls < 2 ? 'running' : 'succeeded' });
  }
  return problem(res, 'not_found', 404);
});
server.listen(PORT, '127.0.0.1', () => console.log(`fixture on ${PORT}`));
