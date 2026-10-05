import { describe, expect, it } from 'vitest';
import * as S from '../../src/api/schemas';

// Panel contract of a REAL engine run (CONTRACT_PANEL_RUN=<run id> CONTRACT_BASE_URL=<server>): the served JSON of the Investigation,
// Alternatives, Gates, Diff and Decision panels validates against the console's zod schemas and keeps the honesty rules.
// Skipped when no run id is given (the fixture world has its own contract suite).
const RUN = process.env.CONTRACT_PANEL_RUN;
const BASE = process.env.CONTRACT_BASE_URL ?? '';
const DEBUG = '/internal/v1/debug';
const get = async (p: string) => { const r = await fetch(BASE + p); return { status: r.status, body: await r.json().catch(() => null) as unknown }; };

describe.skipIf(!RUN)('panels of a real run', () => {
  it('investigation: hypotheses carry statements, evidence refs resolve to shown items, doubles stay visible', async () => {
    const r = await get(`${DEBUG}/runs/${RUN}/investigation`);
    expect(r.status).toBe(200);
    const inv = S.Investigation.parse(r.body);
    expect(inv.hypothesis).toBeTruthy();
    expect(inv.verifier).not.toBe('unknown');
    const ids = new Set(inv.evidence.map((e) => e.evidence_ref.id));
    expect(inv.hypotheses!.length).toBeGreaterThanOrEqual(1);
    for (const h of inv.hypotheses!) {
      expect(h.statement.length).toBeGreaterThan(10);
      for (const ref of h.evidence_refs ?? []) expect(ids.has(ref), `${h.hypothesis_id} -> ${ref}`).toBe(true);
    }
    for (const e of inv.evidence) expect(e.evidence_ref.digest).toMatch(/^[0-9a-f]{64}$/);
    const text = JSON.stringify(inv);
    expect(text).toMatch(/scripted/i);
    expect(text).toMatch(/stand-in|stand_in/);
    expect(inv.evidence.some((e) => e.relation === 'limits')).toBe(true);
  });
  it('gates: native and improvement are separate; the failed improvement says why; the proposal resolves to a diff', async () => {
    const g = S.Gates.parse((await get(`${DEBUG}/runs/${RUN}/gates`)).body);
    expect(g.native.status).not.toBe('not_evaluable');
    expect(g.native.report_ref?.digest).toMatch(/^[0-9a-f]{64}$/);
    expect(g.improvement.status).not.toBe('not_evaluable');
    expect((g.attempts ?? []).length).toBeGreaterThanOrEqual(1);
    if (g.improvement.status !== 'pass') expect(g.improvement.reason_code).toBeTruthy();
    expect(g.proposal_id).toBeTruthy();
    const d = S.Diff.parse((await get(`${DEBUG}/proposals/${encodeURIComponent(g.proposal_id!)}/diff`)).body);
    expect(d.lines.some((l) => l.op === 'add') && d.lines.some((l) => l.op === 'ctx')).toBe(true);
  });
  it('alternatives: do-nothing next to the proposed change, with unknown (never invented) expected effect', async () => {
    const a = S.Alternatives.parse((await get(`${DEBUG}/runs/${RUN}/alternatives`)).body);
    expect(a.items.map((i) => i.kind)).toEqual(['do_nothing', 'proposed_change']);
    expect(a.items.every((i) => i.expected_abandoned == null)).toBe(true);
  });
  it('decision: the card is labelled SIMULATED, offers no command, and states the gate it was taken on', async () => {
    const d = S.RunDecision.parse((await get(`${DEBUG}/runs/${RUN}/decision`)).body);
    expect(d.available_commands).toEqual([]);
    expect(d.card.simulated).toBe(true);
    expect(d.card.label).toBe('SIMULATED');
    expect(d.card.quality_claims).toBe('forbidden');
    expect(d.card.gate.improvement).not.toBeNull();
    expect(d.card.reasons.join(' ')).toMatch(/simulated/i);
  });
  it('graph: no trace id is invented (every node says null), so the traces panel stays degraded and says so', async () => {
    const g = S.Graph.parse((await get(`${DEBUG}/runs/${RUN}/graph`)).body);
    expect(g.nodes.length).toBeGreaterThan(0);
    expect(g.nodes.every((n) => n.trace_id === null)).toBe(true);
  });
});
