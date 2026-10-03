// @ts-expect-error plain ESM fixture module without types
import { SCENARIOS, F_COVERAGE, makeScenario } from '../../fixtures/scenarios.mjs';
import { describe, expect, it } from 'vitest';

type World = {
  runs: Record<string, { state: string; nodes: { node_id: string; status: string; reason_code: string | null }[] }>;
  events: Record<string, unknown[]>;
  gates: { native: { status: string }; improvement: { status: string }; combined: { decision: string } };
  decision: { available_commands: string[] };
  hidden?: Record<string, boolean>;
};
const build = (n: string) => makeScenario(n) as World;
const names = Object.keys(SCENARIOS as object);

describe('fixture scenarios', () => {
  it('cover F01-F25 with an implemented scenario/control or a named gap', () => {
    const ids = Array.from({ length: 25 }, (_, i) => `F${String(i + 1).padStart(2, '0')}`);
    expect(Object.keys(F_COVERAGE as object).sort()).toEqual(ids);
    for (const id of ids) expect(((F_COVERAGE as Record<string, string>)[id] ?? '')).toMatch(/^(scenario|control|gap):/);
    for (const [id, v] of Object.entries(F_COVERAGE as Record<string, string>)) {
      if (v.startsWith('scenario:')) expect(names, id).toContain(v.slice(9));
    }
  });
  it('are deterministic and declare a machine-readable expect', () => {
    for (const n of names) {
      expect(JSON.stringify(build(n)), n).toBe(JSON.stringify(build(n)));
      expect(Object.keys((SCENARIOS as Record<string, { expect: object }>)[n]!.expect).length, n).toBeGreaterThan(0);
    }
  });
  it('satisfy their own expect', () => {
    expect(Object.keys(build('empty').runs)).toHaveLength(0);
    expect(Object.keys(build('positive').runs)).toHaveLength(2);
    expect(build('positive').runs['run-positive']!.nodes.every((x) => x.status === 'complete')).toBe(true);
    const f = build('fail_revise');
    expect([f.gates.native.status, f.gates.combined.decision]).toEqual(['fail', 'revise']);
    const i = build('failed_infra');
    expect([i.gates.native.status, i.gates.improvement.status]).toEqual(['unsafe', 'failed_infra']);
    expect(build('cancel_requested').runs['run-cancel']!.nodes[0]!.status).toBe('running');
    expect(build('cancel_confirmed').runs['run-cancel']!.nodes[0]!.status).toBe('cancelled');
    expect(build('forbidden_cross_tenant').hidden).toEqual({ 'run-other-tenant': true });
    expect(build('bot_vs_human').decision.available_commands).toEqual([]);
    const l = build('large_run');
    expect(Object.keys(l.runs)).toHaveLength(100);
    expect(l.events['run-large-0']).toHaveLength(1200);
  });
});

describe('F11 / F21 / canary scenarios', () => {
  type Nodes = { runs: Record<string, { nodes: { trace_id: string | null }[] }> };
  it('collector_down: every node has trace_id null', () => {
    const nodes = Object.values((build('collector_down') as unknown as Nodes).runs).flatMap((r) => r.nodes);
    expect(nodes.length).toBeGreaterThan(0);
    expect(nodes.every((n) => n.trace_id === null)).toBe(true);
  });
  it('default scenario nodes carry well-formed trace ids', () => {
    for (const n of Object.values((build('default') as unknown as Nodes).runs).flatMap((r) => r.nodes)) expect(n.trace_id).toMatch(/^[0-9a-f]{32}$/);
  });
  it('canary scenario plants CANARY_* markers for the browser leak tests', () => {
    expect(JSON.stringify(build('canary'))).toMatch(/CANARY_SECRET_[0-9a-f-]{36}/);
  });
  it('F11 and F21 are covered, not gaps', () => {
    const cov = F_COVERAGE as Record<string, string>;
    expect(cov.F11).toBe('scenario:collector_down');
    expect(cov.F21).toBe('control:bump_decision');
  });
});
