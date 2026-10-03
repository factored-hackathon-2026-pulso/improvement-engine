// Pure, deterministic fixture world (L7). All data is synthetic; shapes follow plan 16.10 and
// fields beyond it are consumer proposals (marked in the manifest, not on the wire here).
export const T0 = '2026-01-01T00:00:00Z';
export const node =(node_id, label, stage, status, depends_on = [], reason_code = null) => ({
  node_id, label, stage, status, depends_on, reason_code, node_kind: 'material_step', job_ref: null,
});

export function baseWorld() {
  const runs = {
    'run-active': {
      run_id: 'run-active', title: 'Investigation in progress', state: 'running', origin: 'scheduled', revision: 1,
      nodes: [
        node('scout', 'Scout sources', 'scout', 'complete'),
        node('hypothesis', 'Form hypothesis', 'hypothesis', 'running', ['scout']),
        node('verify', 'Verify hypothesis', 'verifier', 'planned', ['hypothesis']),
        node('evaluate', 'Evaluate proposal', 'evaluation', 'planned', ['verify']),
        node('decision', 'Human decision', 'decision', 'planned', ['evaluate']),
      ],
    },
    'run-refuted': {
      run_id: 'run-refuted', title: 'Refuted hypothesis (kept visible)', state: 'completed', origin: 'scheduled', revision: 4,
      nodes: [
        node('scout', 'Scout sources', 'scout', 'complete'),
        node('hypothesis', 'Form hypothesis', 'hypothesis', 'complete', ['scout']),
        node('verify', 'Verify hypothesis', 'verifier', 'complete', ['hypothesis'], 'verifier_refuted'),
        node('decision', 'Do nothing', 'decision', 'complete', ['verify']),
      ],
    },
    'run-blocked': {
      run_id: 'run-blocked', title: 'Blocked and unknown states', state: 'running', origin: 'manual', revision: 2,
      nodes: [
        node('scout', 'Scout sources', 'scout', 'waiting_dependency', [], 'dependency_blocked'),
        node('evaluate', 'Evaluate proposal', 'evaluation', 'unknown', ['scout'], 'model_unknown'),
        node('release', 'Release', 'release', 'unknown', ['evaluate'], 'release_ack_unknown'),
      ],
    },
  };
  const events = {};
  for (const id of Object.keys(runs)) events[id] = [];
  const ev = (id, relation, summary, source_kind, validation, level) => ({
    evidence_ref: { id, digest: id.padEnd(64, 'a').slice(0, 64).replace(/[^0-9a-f]/g, 'a'), media_type: 'application/json' },
    relation, summary, source_kind, validation, available_at: T0, level,
  });
  return {
    runs, events,
    investigation: {
      'run-refuted': {
        hypothesis: 'Raising the retry limit reduces abandoned sessions',
        verifier: 'refuted',
        evidence: [
          ev('ev1', 'supports', 'Abandonment fell 4% after the earlier retry change', 'historical_csv', 'reproducible', null),
          ev('ev2', 'contradicts', 'Holdout cohort shows no change (n=412, interval spans 0)', 'sandbox_evaluated', 'offline_evaluated', 'engine_event'),
          ev('ev3', 'limits', 'Only 2 of 13 sources observed', 'platform_observed', 'unverified', null),
        ],
      },
    },
    diff: {
      proposal_id: 'prop-1',
      lines: [
        { op: 'ctx', text: 'retry_policy:' }, { op: 'del', text: '  max_retries: 2' },
        { op: 'add', text: '  max_retries: 3' }, { op: 'ctx', text: '  backoff: exponential' },
      ],
    },
    gates: {
      native: { status: 'pass', report_ref: null, reason_code: null, checked_at: T0 },
      improvement: { status: 'insufficient_power', reason_code: 'insufficient_power', receipt_refs: [], checked_at: T0 },
      combined: { decision: 'hold', reason_code: 'improvement_gate_not_pass' },
    },
    memory: [
      { memory_id: 'mem-1', title: 'Retry limit is not a lever', status: 'active', revoked: false },
      { memory_id: 'mem-2', title: 'Revoked claim', status: 'tombstone', revoked: true },
    ],
    decision: { decision_id: 'dec-1', available_commands: ['approve', 'reject'], needs_step_up: true, stepped_up: false },
    commands: {},
  };
}
