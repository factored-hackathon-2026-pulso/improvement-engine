import type { z } from 'zod';
import type * as S from '../api/schemas';

type Inv = z.infer<typeof S.Investigation>;
type Attempt = z.infer<typeof S.GateAttempt>;

export interface Hyp { id: string; statement: string; verdict: string; supports: string[]; counter: string[] }

const COMPETING = /^Competing hypothesis (\S+) refuted: ([\s\S]*)$/;

/**
 * One entry per hypothesis with its own verdict. Uses the server's explicit `hypotheses` list when present (R5/M1);
 * otherwise derives it from the single-hypothesis shape: the main hypothesis takes the run verdict, and a contradicting
 * evidence item that names a competing hypothesis becomes that hypothesis, refuted. Nothing is ever invented.
 */
export function hypothesesOf(inv: Inv): Hyp[] {
  const byId = new Map(inv.evidence.map((e) => [e.evidence_ref.id, e]));
  if (inv.hypotheses && inv.hypotheses.length > 0) {
    return inv.hypotheses.map((h) => {
      const ev = (h.evidence_refs ?? []).map((id) => byId.get(id)).filter((e) => e !== undefined);
      return {
        id: h.hypothesis_id, statement: h.statement, verdict: h.verdict,
        supports: ev.filter((e) => e.relation === 'supports').map((e) => e.summary),
        counter: ev.filter((e) => e.relation === 'contradicts').map((e) => e.summary),
      };
    });
  }
  const out: Hyp[] = [];
  const decoys: Hyp[] = [];
  const mainCounter: string[] = [];
  for (const e of inv.evidence) {
    if (e.relation !== 'contradicts') continue;
    const m = COMPETING.exec(e.summary);
    if (m) decoys.push({ id: m[1]!, statement: m[1]!, verdict: 'refuted', supports: [], counter: [m[2]!] });
    else mainCounter.push(e.summary);
  }
  if (inv.hypothesis) {
    out.push({
      id: 'main', statement: inv.hypothesis, verdict: inv.verifier,
      supports: inv.evidence.filter((e) => e.relation === 'supports').map((e) => e.summary), counter: mainCounter,
    });
  }
  return [...out, ...decoys];
}

/** A candidate passes only with an explicit pass on BOTH gates; anything else (fail, unknown, not evaluable, ...) is not a pass. */
export const attemptOutcome = (a: Attempt): 'pass' | 'fail' => (a.native === 'pass' && a.improvement.status === 'pass' ? 'pass' : 'fail');

/** Human-decision hook states: the engine stops here until a human (local identity) acts; the console never advances them. */
export function hookState(n: { status: string; reason_code: string | null }): 'decision_pending' | 'awaiting_authority' | null {
  if (n.status === 'waiting_dependency' && n.reason_code === 'human_decision_pending') return 'decision_pending';
  if (n.status === 'planned' && n.reason_code === 'awaiting_human_authority') return 'awaiting_authority';
  return null;
}
