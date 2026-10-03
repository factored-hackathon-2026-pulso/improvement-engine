import fs from 'node:fs';
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { makeScenario } from '../fixtures/scenarios.mjs';

let scenario = 'default';
let world = makeScenario(scenario);

/** A world JSON (e.g. produced by the demo driver) must carry at least these collections; missing optional ones are defaulted. */
function adoptWorld(w) {
  const isObj = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);
  if (!isObj(w) || !isObj(w.runs) || !isObj(w.decision)) throw new Error('invalid_world');
  for (const r of Object.values(w.runs)) if (!isObj(r) || typeof r.run_id !== 'string' || !Array.isArray(r.nodes)) throw new Error('invalid_world');
  const next = structuredClone(w);
  next.events = isObj(next.events) ? next.events : {};
  for (const id of Object.keys(next.runs)) next.events[id] ??= [];
  next.investigation = isObj(next.investigation) ? next.investigation : {};
  next.memory = Array.isArray(next.memory) ? next.memory : [];
  next.commands = isObj(next.commands) ? next.commands : {};
  next.gates ??= { native: { status: 'unknown', reason_code: null }, improvement: { status: 'unknown', reason_code: null }, combined: { decision: 'hold', reason_code: null } };
  next.diff ??= { proposal_id: 'none', lines: [] };
  return next;
}
if (process.env.FIXTURE_WORLD_FILE) {
  world = adoptWorld(JSON.parse(fs.readFileSync(process.env.FIXTURE_WORLD_FILE, 'utf8')));
  scenario = 'loaded';
}
/** Which runs own the gate/attempt/alternatives data: explicit gates_by_run, else (demo worlds) runs with an evaluation stage, else all. */
const ownsEvaluation = (run) => (world.gates_by_run ? run.run_id in world.gates_by_run : world.demo ? run.nodes.some((n) => n.stage === 'evaluation') : true);
const gatesFor = (run) => {
  if (world.gates_by_run) return world.gates_by_run[run.run_id] ?? null;
  return ownsEvaluation(run) ? world.gates : null;
};
const NOT_EVALUATED = {
  native: { status: 'not_evaluable', reason_code: 'no_evaluation_in_run', checked_at: null },
  improvement: { status: 'not_evaluable', reason_code: 'no_evaluation_in_run', receipt_refs: [], checked_at: null },
  combined: { decision: 'not_applicable', reason_code: 'no_evaluation_in_run' },
};
const freshFaults = () => ({ stream: null, api: null, session: null, stepup: null });
let faults = freshFaults(); // stream: 'gone' (410 once) | 'unauthorized'; api: 'unauthorized'; session|stepup: 'down' (503)
const hb = { ms: 5000, muted: false }; // SSE heartbeat cadence; muted simulates a silent but connected stream
// Opaque loopback-HTTP session cookie (plan 16.13.6): pulso_local_session, never __Host- over plain HTTP.
const SESSION_COOKIE = `pulso_local_session=${randomUUID().replaceAll('-', '')}; HttpOnly; SameSite=Lax; Path=/`;
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
const send = (res, code, body, headers = {}) => {
  res.writeHead(code, { 'content-type': 'application/json', 'cache-control': 'no-store', ...headers });
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

function emit(runId, nodeId, status, deliver = true) {
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
  if (deliver) for (const c of sseClients) if (c.runId === runId) c.res.write(`id: ${ev.sequence}\ndata: ${JSON.stringify(ev)}\n\n`);
  return ev;
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://x');
  const p = url.pathname;
  const m = req.method;
  if (p === '/healthz') return send(res, 200, { ok: true });
  if (p.startsWith('/__fixture/')) {
    if (p === '/__fixture/reset' || p === '/__fixture/scenario') {
      const b = (await readBody(req)) ?? {};
      const next = p === '/__fixture/reset' ? (b.scenario ?? 'default') : b.scenario;
      try { world = makeScenario(next); scenario = next; } catch { return send(res, 400, { code: 'unknown_scenario' }); }
      for (const c of sseClients) c.res.end();
      sseClients.clear();
      faults = freshFaults();
      hb.ms = 5000; hb.muted = false;
      return send(res, 200, { ok: true, scenario });
    }
    if (p === '/__fixture/load') { // replace the whole world with the posted JSON (demo driver output)
      const b = await readBody(req);
      try { world = adoptWorld(b); scenario = 'loaded'; } catch { return problem(res, 'invalid_world', 400); }
      for (const c of sseClients) c.res.end();
      sseClients.clear();
      faults = freshFaults();
      hb.ms = 5000; hb.muted = false;
      return send(res, 200, { ok: true, scenario, runs: Object.keys(world.runs) });
    }
    if (p === '/__fixture/deliver') { // push existing log entries as raw SSE frames, in the given order (dup/reorder/gap)
      const b = await readBody(req);
      const list = b && world.events[b.run_id];
      if (!list) return send(res, 400, { code: 'bad_deliver' });
      for (const seq of b.sequences) {
        const ev = list.find((e) => e.sequence === seq);
        if (ev) for (const c of sseClients) if (c.runId === b.run_id) c.res.write(`id: ${ev.sequence}\ndata: ${JSON.stringify(ev)}\n\n`);
      }
      return send(res, 200, { ok: true });
    }
    if (p === '/__fixture/emit') {
      const b = await readBody(req);
      const ev = b && emit(b.run_id, b.node_id, b.status, b.deliver !== 'none');
      return ev ? send(res, 200, ev) : send(res, 400, { code: 'bad_emit' });
    }
    if (p === '/__fixture/fault') {
      const b = await readBody(req);
      const allowed = { stream: ['gone', 'unauthorized'], api: ['unauthorized'], session: ['down'], stepup: ['down'] };
      const ok = b && Object.keys(b).every((k) => k in allowed && (b[k] === null || allowed[k].includes(b[k])));
      if (!ok) return send(res, 400, { code: 'bad_fault' });
      for (const k of Object.keys(b)) faults[k] = b[k];
      return send(res, 200, { ok: true });
    }
    if (p === '/__fixture/heartbeat') {
      const b = await readBody(req);
      if (!b || (b.ms !== undefined && !(Number.isInteger(b.ms) && b.ms >= 50)) || (b.muted !== undefined && typeof b.muted !== 'boolean')) return send(res, 400, { code: 'bad_heartbeat' });
      if (b.ms !== undefined) hb.ms = b.ms;
      if (b.muted !== undefined) hb.muted = b.muted;
      return send(res, 200, { ok: true, ...hb });
    }
    if (p === '/__fixture/bump_decision') { // another actor changed the decision target: its domain revision moves
      world.decision.revision += 1;
      return send(res, 200, { revision: world.decision.revision });
    }
    if (p === '/__fixture/cut') { // simulate a dropped transport / API restart: end every open stream
      for (const c of sseClients) c.res.end();
      sseClients.clear();
      return send(res, 200, { ok: true });
    }
    if (p === '/__fixture/state') return send(res, 200, world);
    return send(res, 404, { code: 'not_found' });
  }
  if (faults.api === 'unauthorized' && (p.startsWith(PREFIX) || p === '/api/v1/auth/step-up')) return problem(res, 'session_expired', 401);
  if (p === '/api/v1/auth/session') {
    if (faults.session === 'down') return problem(res, 'dependency_unavailable', 503);
    return send(res, 200, {
      principal: 'fixture-human', tenant_id: 'tenant-fixture', scopes: ['debug/read', 'evolution/decide'],
      expires_at: '2099-01-01T00:00:00Z', csrf_token: CSRF,
      auth: { simulated: true, level: world.decision.stepped_up ? 'step_up' : 'basic', auth_at: '2026-01-01T00:00:00Z' },
    }, { 'set-cookie': SESSION_COOKIE });
  }
  if (p === '/api/v1/auth/step-up' && m === 'POST') {
    if (req.headers['x-csrf-token'] !== CSRF) return problem(res, 'csrf_failed', 403);
    if (faults.stepup === 'down') return problem(res, 'dependency_unavailable', 503);
    world.decision.stepped_up = true;
    return send(res, 200, { level: 'step_up' });
  }
  if (p === `${PREFIX}/profile`) {
    const d = world.demo;
    if (d) {
      return send(res, 200, {
        target: 'demo-standin', runtime_profile: d.runtime_profile ?? 'fixture', doubles: d.doubles ?? ['api_fixture'], pin: null,
        ...(d.mode ? { mode: d.mode } : {}), ...(d.decision_hook ? { decision_hook: d.decision_hook } : {}),
        ...(d.doubles_detail ? { doubles_detail: d.doubles_detail } : {}),
      });
    }
    return send(res, 200, { target: 'mock', runtime_profile: 'fixture', doubles: ['api_fixture', 'sse_fixture', 'identity_fixture'], pin: null });
  }
  if (p === `${PREFIX}/runs`) {
    const items = Object.values(world.runs).map((r) => ({
      run_id: r.run_id, title: r.title, state: r.state, origin: r.origin, projection_revision: r.revision,
    }));
    return send(res, 200, { ...env(null, 1, 'ok'), items, next_cursor: null });
  }
  let mm = p.match(new RegExp(`^${PREFIX}/runs/([^/]+)/(graph|events|events/stream|investigation|gates|alternatives)$`));
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
    if (kind === 'gates') {
      const g = gatesFor(run);
      if (!g) return send(res, 200, { ...env(ref, run.revision, 'ok'), ...NOT_EVALUATED, proposal_id: null, attempts: [] });
      return send(res, 200, {
        ...env(ref, run.revision, 'ok'), ...g,
        proposal_id: g.proposal_id ?? world.diff.proposal_id ?? null, attempts: g.attempts ?? (world.gates_by_run ? [] : (world.demo?.attempts ?? [])),
      });
    }
    if (kind === 'alternatives') {
      const items = ownsEvaluation(run) ? (world.demo?.alternatives ?? []) : [];
      return send(res, 200, { ...env(ref, run.revision, 'ok'), items });
    }
    if (faults.stream === 'unauthorized') return problem(res, 'session_expired', 401);
    if (faults.stream === 'gone') {
      faults.stream = null; // 410 body per CLQ-24: Problem{code:cursor_expired, current_ref, recovery_after_sequence, snapshot_url}
      return problem(res, 'cursor_expired', 410, {
        current_ref: ref, recovery_after_sequence: world.events[run.run_id].length, snapshot_url: `${PREFIX}/runs/${run.run_id}/graph`,
      });
    }
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' });
    res.write(': open\n\n');
    const resumeFrom = Number(req.headers['last-event-id'] ?? 0);
    if (resumeFrom > 0) {
      for (const e of world.events[run.run_id]) if (e.sequence > resumeFrom) res.write(`id: ${e.sequence}\ndata: ${JSON.stringify(e)}\n\n`);
    }
    const c ={ res, runId: run.run_id };
    sseClients.add(c);
    const hbTimer = setInterval(() => { if (!hb.muted) res.write(': hb\n\n'); }, hb.ms);
    req.on('close', () => { clearInterval(hbTimer); sseClients.delete(c); });
    return undefined;
  }
  mm = p.match(new RegExp(`^${PREFIX}/proposals/([^/]+)/diff$`));
  if (mm) {
    const id = decodeURIComponent(mm[1]);
    const d = world.diffs?.[id] ?? (world.diff.proposal_id === id ? world.diff : null);
    if (!d) return problem(res, 'not_found', 404);
    return send(res, 200, { ...env({ kind: 'proposal', id }, 1, 'ok'), ...d });
  }
  if (p === `${PREFIX}/memory`) return send(res, 200, { ...env(null, 1, 'ok'), items: world.memory });
  if (p === `${PREFIX}/decisions/dec-1`) {
    return send(res, 200, {
      ...env({ kind: 'decision', id: 'dec-1' }, 1, 'pending', { available_commands: world.decision.available_commands }),
      needs_step_up: world.decision.needs_step_up && !world.decision.stepped_up, domain_revision: world.decision.revision,
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
    if (!Number.isInteger(b.expected_revision)) {
      return problem(res, 'validation_error', 422, { field_errors: [{ field: 'expected_revision', code: 'required_integer' }] });
    }
    if (b.expected_revision !== world.decision.revision) { // CAS on the decision's domain revision, never a silent overwrite
      return problem(res, 'stale_revision', 409, {
        conflict: { expected_revision: b.expected_revision, current_revision: world.decision.revision, diff_ref: null },
      });
    }
    if (world.decision.needs_step_up && !world.decision.stepped_up) return problem(res, 'waiting_human_reauthentication', 403);
    const id = `cmd-${key}`;
    const fingerprint = JSON.stringify(b);
    if (world.commands[id] && world.commands[id].fingerprint !== fingerprint) return problem(res, 'idempotency_conflict', 409);
    world.commands[id] ??= { polls: 0, fingerprint };
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
server.listen(PORT, process.env.FIXTURE_HOST ?? '127.0.0.1', () => console.log(`fixture on ${PORT}`));
