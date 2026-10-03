import { describe, expect, it } from 'vitest';
import { CAPABILITIES, DEMO_SOURCES, FAMILIES, eligibility, insightsOf, type SourceId } from '../../src/features/sourcesModel';

const src = (id: SourceId) => DEMO_SOURCES.find((s) => s.id === id)!;
const fam = (id: SourceId, f: string) => eligibility(src(id)).find((e) => e.family === f)!;

describe('sources demo model', () => {
  it('has the three sources, all marked stand_in (demo model, not an API)', () => {
    expect(DEMO_SOURCES.map((s) => s.id)).toEqual(['e0_enriched', 'bank_original', 'platform_live']);
    for (const s of DEMO_SOURCES) expect(s.provenance).toBe('stand_in');
  });
  it('uses the exporter capability vocabulary verbatim', () => {
    for (const c of ['origin', 'topic', 'complaint_id', 'identity_check', 'routing_step', 'tool_call', 'approval', 'suggestion', 'case_close.resolved', 'csat']) expect(CAPABILITIES).toContain(c);
    expect(src('platform_live').profileVersion).toBe('platform_live.phase1/1');
    expect(src('platform_live').phase).toBe(1);
    expect(src('platform_live').channels).toEqual(['app_chat', 'web_chat']);
  });
  it('PL-06: a tool detector is unsupported on platform_live naming tool_call, and eligible on the 0.5.1 superset', () => {
    expect(fam('platform_live', 'tool_failure')).toMatchObject({ status: 'unsupported', missing: ['tool_call'] });
    expect(fam('e0_enriched', 'tool_failure')).toMatchObject({ status: 'eligible', missing: [] });
  });
  it('human-resolution families stop at insufficient_human_evidence naming the missing capability', () => {
    expect(fam('platform_live', 'human_resolution_verified')).toMatchObject({ status: 'insufficient_human_evidence', missing: ['case_close.resolved', 'csat'] });
  });
  it('operational families are eligible on platform_live and need no AI-layer capability', () => {
    for (const f of FAMILIES.filter((x) => x.kind === 'operational')) expect(fam('platform_live', f.id).status).toBe('eligible');
  });
  it('every non-eligible family names at least one missing capability', () => {
    for (const s of DEMO_SOURCES) for (const e of eligibility(s)) if (e.status !== 'eligible') expect(e.missing.length, `${s.id}/${e.family}`).toBeGreaterThan(0);
  });
  it('platform_live counters use the exporter vocabulary; other sources have no exporter', () => {
    const c = src('platform_live').counters!;
    expect(Object.keys(c)).toEqual(['events_exported', 'unknown_event_type', 'denied_event_type', 'gap_suspected', 'late_event', 'window_revisions', 'team_generated_excluded']);
    expect(src('e0_enriched').counters).toBeNull();
    expect(src('bank_original').counters).toBeNull();
  });
  it('PL-10: insights only stop at insufficient_* / waiting_dependency, never a publish state', () => {
    const ins = insightsOf(src('platform_live'));
    expect(ins.length).toBeGreaterThan(0);
    for (const i of ins) {
      expect(i.status).toMatch(/^(insufficient_[a-z_]+|waiting_dependency)$/);
      expect(i.evidenceKind).toBe('observed');
    }
    expect(insightsOf(src('e0_enriched'))).toEqual([]);
  });
});
