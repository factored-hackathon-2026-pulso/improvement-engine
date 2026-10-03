import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import * as S from '../../src/api/schemas';
import { attemptOutcome, hookState, hypothesesOf } from '../../src/features/demoModel';
import { bannerLevel } from '../../src/app/banner';

const world = JSON.parse(readFileSync('fixtures/demo-world.json', 'utf8'));
const env = { schema_version: '1', tenant_id: 't', projection_revision: 1, status: 'ok', blocking_reasons: [], available_commands: [] };
const inv = (id: string) => S.Investigation.parse({ ...env, ...world.investigation[id] });

describe('hypothesesOf', () => {
  it('derives the main hypothesis and the refuted decoys with their counterevidence from the demo investigation', () => {
    const hs = hypothesesOf(inv('run-demo'));
    expect(hs[0]).toMatchObject({ id: 'main', verdict: 'supported' });
    expect(hs[0]!.supports.length).toBeGreaterThan(0);
    const decoys = hs.filter((h) => h.id !== 'main');
    expect(decoys.map((h) => h.id)).toEqual(['card_replacement/otp_verify', 'card_replacement/mobile']);
    for (const d of decoys) {
      expect(d.verdict).toBe('refuted');
      expect(d.counter.join(' ')).toMatch(/weeks only/);
    }
  });
  it('keeps a refuted main hypothesis refuted, with its counterevidence', () => {
    const hs = hypothesesOf(inv('run-demo-refuted'));
    expect(hs).toHaveLength(1);
    expect(hs[0]).toMatchObject({ id: 'main', verdict: 'refuted' });
    expect(hs[0]!.counter.length).toBeGreaterThan(0);
  });
  it('uses the explicit hypotheses list when the server provides it', () => {
    const base = inv('run-demo');
    const hs = hypothesesOf({ ...base, hypotheses: [{ hypothesis_id: 'h1', statement: 'A', verdict: 'unknown', evidence_refs: [base.evidence[0]!.evidence_ref.id] }] });
    expect(hs).toEqual([{ id: 'h1', statement: 'A', verdict: 'unknown', supports: [base.evidence[0]!.summary], counter: [] }]);
  });
  it('a missing hypothesis yields no entries, never an invented one', () => {
    expect(hypothesesOf({ ...inv('run-demo'), hypothesis: null, evidence: [] })).toEqual([]);
  });
});

describe('attemptOutcome: a failed attempt is never shown as pass', () => {
  const [a1, a2] = world.demo.attempts.map((a: unknown) => S.GateAttempt.parse(a));
  it('candidate 1 failed the improvement gate even though the native gate passed', () => {
    expect(attemptOutcome(a1)).toBe('fail');
    expect(attemptOutcome(a2)).toBe('pass');
  });
  it('anything that is not an explicit pass on both gates is not a pass', () => {
    expect(attemptOutcome({ ...a2, native: 'unknown' })).toBe('fail');
    expect(attemptOutcome({ ...a2, improvement: { ...a2.improvement, status: 'insufficient_power' } })).toBe('fail');
    expect(attemptOutcome({ ...a2, native: 'pass', improvement: { status: 'pass' } })).toBe('pass');
  });
});

describe('hookState', () => {
  it('waiting_dependency + human_decision_pending and planned + awaiting_human_authority are human-decision hook states', () => {
    expect(hookState({ status: 'waiting_dependency', reason_code: 'human_decision_pending' })).toBe('decision_pending');
    expect(hookState({ status: 'planned', reason_code: 'awaiting_human_authority' })).toBe('awaiting_authority');
  });
  it('other combinations are not hook states', () => {
    expect(hookState({ status: 'waiting_dependency', reason_code: 'dependency_blocked' })).toBeNull();
    expect(hookState({ status: 'planned', reason_code: null })).toBeNull();
    expect(hookState({ status: 'complete', reason_code: 'human_decision_pending' })).toBeNull();
  });
});

describe('banner level for the stand-in demo', () => {
  const p = { target: 'demo-standin', runtime_profile: 'real_local_core_standin', doubles: ['stand_in_engine'], pin: null, mode: 'stand_in' };
  it('a stand-in profile is level stand_in on a fixture client, and never verified/green even if the client claims real', () => {
    expect(bannerLevel('fixture', p)).toBe('stand_in');
    expect(bannerLevel('real', p)).toBe('unverified');
  });
});
