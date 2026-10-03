// L7 fixture scenarios (plan 17.3.7 F01-F25). Pure and deterministic: each scenario patches the base
// world and declares a machine-readable `expect`, which tests/unit/scenarios.test.ts verifies.
import { baseWorld, node, T0 } from './world.mjs';

const mkEvents = (runId, n, nodeId) => Array.from({ length: n }, (_, i) => ({
  event_id: `00000000-0000-4000-8000-${String(i + 1).padStart(12, '0')}`, run_id: runId, sequence: i + 1,
  entity_ref: { kind: 'node', id: nodeId }, projection_revision: i + 1, kind: 'node_status_changed',
}));
const run = (run_id, title, state, revision, nodes) => ({ run_id, title, state, origin: 'scheduled', revision, nodes });
const CANARY_ID = '123e4567-e89b-42d3-a456-426614174000';
const POISON_JWS = 'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJlXzEyMzQ1';
const evidence = (id, summary) => ({
  evidence_ref: { id, digest: 'a'.repeat(64), media_type: 'text/plain' }, relation: 'supports', summary,
  source_kind: 'historical_csv', validation: 'reproducible', available_at: T0,
});
const only = (w, r) => { w.runs = r; w.events = Object.fromEntries(Object.keys(r).map((k) => [k, []])); };

export const SCENARIOS = {
  default: { fids: ['F02', 'F03', 'F05', 'F08', 'F09', 'F10', 'F14', 'F15', 'F16', 'F17', 'F18', 'F19', 'F22'], expect: { runs: 3 }, patch: () => undefined },
  empty: { fids: ['F01'], expect: { runs: 0 }, patch: (w) => only(w, {}) },
  positive: {
    fids: ['F04'], expect: { runs: 2, completedNodes: 10, successorOf: 'run-positive' },
    patch: (w) => {
      const done = ['scout', 'hypothesis', 'verify', 'evaluate', 'decision', 'approve', 'publish', 'release', 'observation', 'memory']
        .map((id, i, a) => node(id, id, id, 'complete', i ? [a[i - 1]] : []));
      only(w, {
        'run-positive': run('run-positive', 'Full path to successor run', 'completed', 10, done),
        'run-positive-next': run('run-positive-next', 'Successor run', 'running', 1, [node('scout', 'Scout sources', 'scout', 'running')]),
      });
    },
  },
  fail_revise: {
    fids: ['F06'], expect: { gate: { native: 'fail', combined: 'revise' }, node: { evaluate: 'dead' } },
    patch: (w) => {
      only(w, { 'run-fail': run('run-fail', 'Native gate failed, proposal revised', 'running', 3, [
        node('evaluate', 'Evaluate proposal', 'evaluation', 'dead', [], 'gate_failed'),
        node('revise', 'Revise proposal', 'proposal', 'queued', ['evaluate'], 'proposal_revised'),
      ]) });
      w.gates = {
        native: { status: 'fail', report_ref: null, reason_code: 'gate_failed', checked_at: T0 },
        improvement: { status: 'not_evaluable', reason_code: 'native_gate_failed', receipt_refs: [], checked_at: T0 },
        combined: { decision: 'revise', reason_code: 'native_gate_failed' },
      };
    },
  },
  failed_infra: {
    fids: ['F07'], expect: { gate: { native: 'unsafe', improvement: 'failed_infra', combined: 'hold' } },
    patch: (w) => {
      only(w, { 'run-infra': run('run-infra', 'Infrastructure failure, not a verdict', 'running', 2, [
        node('evaluate', 'Evaluate proposal', 'evaluation', 'dead', [], 'failed_infra'),
      ]) });
      w.gates = {
        native: { status: 'unsafe', report_ref: null, reason_code: 'unsafe', checked_at: T0 },
        improvement: { status: 'failed_infra', reason_code: 'failed_infra', receipt_refs: [], checked_at: T0 },
        combined: { decision: 'hold', reason_code: 'improvement_gate_not_pass' },
      };
    },
  },
  cancel_requested: {
    fids: ['F12'], expect: { runState: 'cancel_requested', node: { evaluate: 'running' }, reason: 'cancel_requested' },
    patch: (w) => only(w, { 'run-cancel': run('run-cancel', 'Cancel requested, effect in flight', 'cancel_requested', 2, [
      node('evaluate', 'Evaluate proposal', 'evaluation', 'running', [], 'cancel_requested'),
    ]) }),
  },
  cancel_confirmed: {
    fids: ['F13'], expect: { runState: 'cancelled', node: { evaluate: 'cancelled' } },
    patch: (w) => only(w, { 'run-cancel': run('run-cancel', 'Cancel confirmed', 'cancelled', 3, [
      node('evaluate', 'Evaluate proposal', 'evaluation', 'cancelled', [], null),
    ]) }),
  },
  forbidden_cross_tenant: {
    fids: ['F20'], expect: { runs: 1, hidden: ['run-other-tenant'] },
    patch: (w) => {
      only(w, { 'run-mine': run('run-mine', 'My tenant run', 'running', 1, [node('scout', 'Scout sources', 'scout', 'running')]) });
      w.hidden = { 'run-other-tenant': true }; // server answers exactly like an unknown id
    },
  },
  poison_notes: {
    fids: ['F23'], expect: { runs: 2, renders: 'text_only', objectNotes: 'rejected' },
    patch: (w) => {
      const one = [node('scout', 'Scout sources', 'scout', 'complete')];
      only(w, {
        'run-poison': run('run-poison', 'Poison notes as text', 'completed', 1, one),
        'run-poison-object': run('run-poison-object', 'Non-string note', 'completed', 1, one),
      });
      w.investigation = {
        'run-poison': {
          hypothesis: '<img src=x onerror="window.__xss=1">', verifier: 'unknown',
          evidence: [evidence('p1', '<script>window.__xss=1</script>'), evidence('p2', `token ${POISON_JWS} leaked`), evidence('p3', '')],
        },
        'run-poison-object': { hypothesis: 'h', verifier: 'unknown', evidence: [evidence('p4', { nested: 'object' })] },
      };
    },
  },
  collector_down: {
    fids: ['F11'], expect: { traceIds: 'null', durableEvents: 'intact' },
    patch: (w) => {
      only(w, { 'run-collector': run('run-collector', 'Collector down: durable timeline only', 'running', 2, [
        node('scout', 'Scout sources', 'scout', 'complete'),
        node('hypothesis', 'Form hypothesis', 'hypothesis', 'running', ['scout']),
      ]) });
      for (const n of w.runs['run-collector'].nodes) n.trace_id = null;
    },
  },
  canary: {
    fids: [], expect: { canary: 'CANARY_SECRET' },
    patch: (w) => {
      only(w, { 'run-canary': run('run-canary', 'Canary markers must never persist', 'completed', 1, [node('scout', 'Scout sources', 'scout', 'complete')]) });
      w.investigation = { 'run-canary': {
        hypothesis: `leaked CANARY_SECRET_${CANARY_ID} in a hypothesis`, verifier: 'unknown',
        evidence: [evidence('c1', `summary with CANARY_SECRET_${CANARY_ID} and ${POISON_JWS}`)],
      } };
    },
  },
  bot_vs_human: {
    fids: ['F25'], expect: { availableCommands: [] },
    patch: (w) => { w.decision.available_commands = []; w.decision.needs_step_up = false; },
  },
  large_run: {
    fids: ['F24'], expect: { runs: 100, events: { 'run-large-0': 1200 } },
    patch: (w) => {
      w.runs = {}; w.events = {};
      for (let i = 0; i < 100; i += 1) {
        const id = `run-large-${i}`;
        w.runs[id] = run(id, `Large run ${i}`, 'running', i === 0 ? 1200 : 1, [node('scout', 'Scout sources', 'scout', 'running')]);
        w.events[id] = i === 0 ? mkEvents(id, 1200, 'scout') : [];
      }
    },
  },
};

/** Plan 17.3.7 F01-F25: how each is covered by the fixture, or the precise gap. */
export const F_COVERAGE = {
  F01: 'scenario:empty', F02: 'scenario:default', F03: 'scenario:default', F04: 'scenario:positive', F05: 'scenario:default',
  F06: 'scenario:fail_revise', F07: 'scenario:failed_infra', F08: 'scenario:default', F09: 'scenario:default',
  F10: 'scenario:default', F11: 'scenario:collector_down', F12: 'scenario:cancel_requested', F13: 'scenario:cancel_confirmed',
  F14: 'control:deliver', F15: 'control:deliver', F16: 'control:deliver', F17: 'control:fault_gone', F18: 'control:cut',
  F19: 'control:fault_unauthorized', F20: 'scenario:forbidden_cross_tenant', F21: 'control:bump_decision',
  F22: 'scenario:default', F23: 'scenario:poison_notes', F24: 'scenario:large_run', F25: 'scenario:bot_vs_human',
};

export function makeScenario(name = 'default') {
  const sc = SCENARIOS[name];
  if (!sc) throw new Error(`unknown scenario: ${name}`);
  const w = baseWorld();
  sc.patch(w);
  return w;
}
