// /internal/v1/automation on the fixture server: golden payloads captured from the Rust read model (debug-api) over the
// simulator's SIM-ONLY draft stream, so `npm run fixture` + `npm run dev` shows #/automatizacion without the Rust server.
// Stateful only for the SIMULATED approve / publish-staging path. PUT /config validates and echoes (no recompute).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'fixtures', 'automation');
const load = (f) => JSON.parse(fs.readFileSync(path.join(DIR, f), 'utf8'));
export const AUTOMATION_PREFIX = '/internal/v1/automation';
const ADMIN = process.env.FIXTURE_ADMIN_TOKEN ?? 'adm';
const BOUNDS = { repeat_q_min_cases: [1, 1e6, true], draft_window: [1, 1e6, true], k_min: [10, 1e6, true], tool_use_min: [0, 1, false], draft_accept_min: [0, 1, false] };

let state = {};
let config = { revision: 0, thresholds: null };
export const resetAutomation = () => { state = {}; config = { revision: 0, thresholds: null }; };

export async function handleAutomation({ req, res, p, m, send, problem, readBody, CSRF }) {
  if (!p.startsWith(AUTOMATION_PREFIX)) return false;
  const rest = p.slice(AUTOMATION_PREFIX.length).replace(/^\//, '').split('/');
  const list = load('case-types.json');
  config.thresholds ??= list.thresholds;
  if (m === 'PUT' && rest.join('/') === 'config') {
    if (req.headers.authorization !== `Bearer ${ADMIN}`) { problem(res, 'unauthorized', 401); return true; }
    const b = (await readBody(req)) ?? {};
    const next = { ...config.thresholds };
    for (const [k, v] of Object.entries(b)) {
      const bound = BOUNDS[k];
      if (!bound || typeof v !== 'number' || v < bound[0] || v > bound[1] || (bound[2] && !Number.isInteger(v)) || (!bound[2] && v <= 0)) { problem(res, 'validation_error', 422); return true; }
      next[k] = v;
    }
    config = { revision: config.revision + 1, thresholds: next };
    send(res, 200, config);
    return true;
  }
  if (m === 'GET' && rest.length === 1 && rest[0] === 'case-types') { send(res, 200, { ...list, thresholds: config.thresholds, revision: config.revision }); return true; }
  if (m === 'GET' && rest.length === 2 && rest[0] === 'case-types') {
    if (!list.case_types.some((c) => c.type_id === rest[1])) { problem(res, 'not_found', 404); return true; }
    const d = load(`detail-${rest[1]}.json`);
    if (d.proposal) d.proposal.state = state[rest[1]] ?? 'proposed';
    send(res, 200, d);
    return true;
  }
  if (m === 'POST' && rest.length === 4 && rest[0] === 'case-types' && rest[2] === 'proposal' && ['approve', 'publish-staging'].includes(rest[3])) {
    if (req.headers['x-csrf-token'] !== CSRF) { problem(res, 'csrf_failed', 403); return true; }
    const id = rest[1];
    const d = list.case_types.some((c) => c.type_id === id) ? load(`detail-${id}.json`) : null;
    if (!d?.proposal) { problem(res, 'not_found', 404); return true; }
    const b = (await readBody(req)) ?? {};
    const cur = state[id] ?? 'proposed';
    const approve = rest[3] === 'approve';
    if (cur !== (approve ? 'proposed' : 'approved_simulated')) { problem(res, approve ? 'already_decided' : 'approval_required', 409); return true; }
    if (approve && b.candidate_hash !== d.proposal.candidate_hash) { problem(res, 'candidate_hash_mismatch', 409); return true; }
    state[id] = approve ? 'approved_simulated' : 'staged_simulated';
    send(res, 200, { state: state[id], simulated: true, proposal_id: d.proposal.proposal_id });
    return true;
  }
  problem(res, 'not_found', 404);
  return true;
}
