// Data for the in-process providers. `fixtureWorld` is a small hand-written, deterministic world for tests and offline UX work.
// `standInWorld` maps the demo model (fixtures/demo-world.json: output of the stand-in demo driver) onto the spec 25 DTOs.
import demo from '../../../fixtures/demo-world.json';
import { ALL_COMMAND_SCOPES, type RunRecord, type World } from './backend';

const T0 = '2026-01-01T00:00:00Z';
const node = (node_id: string, label: string, stage: string, status: string, depends_on: string[] = [], reason_code: string | null = null) =>
  ({ node_id, label, stage, status, depends_on, reason_code, node_kind: 'material_step', trace_id: null });
const ev = (run: string, sequence: number, stage: string, event_code: string, status = 'ok') =>
  ({ sequence, run_ref: run, stage, event_code, status, occurred_at: T0 });

function fixtureRun(id: string, title: string, state: string, origin: string): RunRecord {
  return {
    summary: { run_id: id, title, state, origin }, revision: 3,
    nodes: [node('trigger', 'Trigger accepted', 'trigger', 'complete'), node('scan', 'Scan', 'scan', 'complete', ['trigger']), node('investigate', 'Investigation', 'investigation', state === 'running' ? 'active' : 'complete', ['scan'])],
    events: [ev(id, 1, 'trigger', 'trigger_accepted'), ev(id, 2, 'scan', 'job_completed'), ev(id, 3, 'investigation', 'model_invoked')],
    modelCalls: [{
      call_id: `${id}-mc-1`, run_ref: id, purpose: 'investigate', route: 'fixture-route', judgment_type: 'schema:hypothesis', input_digest: 'sha256:aaaa', output_digest: 'sha256:bbbb',
      payload_redacted: true, validation: 'valid', tokens_in: 120, tokens_out: 40, usd: 0.002, latency_ms: 850, outcome: 'ok', error_code: null, decision_ref: null,
    }],
    queries: [
      { query_id: `${id}-q-1`, run_ref: id, sql_sanitized: 'SELECT count(*) FROM <table> WHERE <filter>', sql_digest: 'sha256:cccc', param_classes: ['date'], rows: 1, bytes: 8, duration_ms: 12, truncated: false, outcome: 'completed', quality_findings: [] },
      { query_id: `${id}-q-2`, run_ref: id, sql_sanitized: null, sql_digest: 'sha256:dddd', param_classes: [], rows: null, bytes: null, duration_ms: null, truncated: false, outcome: 'denied', quality_findings: ['column_not_allowed'] },
    ],
    evals: [{
      candidate_hash: 'sha256:eeee', suite: { id: 'suite-1', version: '1', digest: 'sha256:ffff' }, base_staging: null, baseline_runtime: null,
      core_gate: { status: 'pass' }, improvement_gate: { status: 'unknown', reason_code: 'insufficient_power' }, final: null,
    }],
    memoryDiff: { head_ref: 'head-1', pages_read: [{ page_ref: 'wiki/a', digest: 'sha256:1111' }], proposed: [{ op: 'add', text: 'note' }], published: [], lint: [], cas_lost: false, revocations: [] },
    externalCommands: [{ command_ref: 'ext-1', kind: 'release_sent', state: 'ack_unknown', requested_at: T0, confirmed_at: null, observation_ref: null }],
    jobs: [`${id}-job-1`], availableCommands: [],
  };
}

export const fixtureWorld = (): World => ({
  tenant: 'tenant-fixture', principal: 'fixture-operator', mode: { target: 'fixture', runtime_profile: 'fixture', doubles: ['api_fixture'] },
  scopes: ALL_COMMAND_SCOPES,
  runs: [fixtureRun('run-fx-1', 'Discovery run (fixture)', 'running', 'scheduled'), fixtureRun('run-fx-2', 'Refuted hypothesis (fixture)', 'completed', 'manual')],
  health: [
    { name: 'core', status: 'ok', checked_at: T0 }, { name: 'otel', status: 'degraded', checked_at: T0, reason_code: 'trace_unavailable' },
    { name: 'external_platform', status: 'unknown', checked_at: null, reason_code: 'no_verifiable_health' },
  ],
});

interface DemoNode { node_id: string; label: string; stage: string; status: string; depends_on: string[]; reason_code: string | null; node_kind: string }
interface DemoEvent { run_id: string; sequence: number; entity_ref: { kind: string; id: string }; kind: string }
interface DemoRun { run_id: string; title: string; state: string; origin: string; revision: number; nodes: DemoNode[] }
const demoRuns = demo.runs as unknown as Record<string, DemoRun>;
const demoEvents = demo.events as unknown as Record<string, DemoEvent[]>;

/** The stand-in only has what the demo driver produced: graph, events and mode. No model calls/queries/evals are invented for it. */
export function standInWorld(): World {
  const runs: RunRecord[] = Object.values(demoRuns).map((r) => {
    const stageOf = new Map(r.nodes.map((n) => [n.node_id, n.stage]));
    return {
      summary: { run_id: r.run_id, title: r.title, state: r.state, origin: r.origin }, revision: r.revision,
      nodes: r.nodes.map((n) => ({ ...n, trace_id: null })),
      events: (demoEvents[r.run_id] ?? []).map((e) => ({
        sequence: e.sequence, run_ref: r.run_id, stage: stageOf.get(e.entity_ref.id) ?? e.entity_ref.kind, event_code: e.kind, status: 'recorded', occurred_at: T0,
      })),
      modelCalls: [], queries: [], evals: [],
      memoryDiff: { head_ref: null, pages_read: [], proposed: [], published: [], lint: [], cas_lost: false, revocations: [] },
      externalCommands: [], jobs: [],
      availableCommands: [], // the stand-in engine accepts no debug commands
    };
  });
  const d = demo.demo as { runtime_profile: string; doubles: string[] };
  return {
    tenant: 'tenant-standin', principal: 'standin-viewer', mode: { target: 'demo-standin', runtime_profile: d.runtime_profile, doubles: d.doubles },
    scopes: [], runs,
    health: [{ name: 'engine', status: 'unknown', checked_at: null, reason_code: 'stand_in_no_health' }],
  };
}
