// Demo model for the Sources view. Every value here is a stand_in: it mirrors the vocabulary of platform-exporter
// (profile.py capabilities, service.py finding/counter names) but is NOT read from a real API.
export type SourceId = 'e0_enriched' | 'bank_original' | 'platform_live';
export const CAPABILITIES = [
  'origin', 'topic', 'complaint_id', 'identity_check', 'routing_step', 'copilot_query', 'tool_call', 'approval',
  'suggestion', 'case_close.resolved', 'csat', 'teams',
] as const;
export type Capability = (typeof CAPABILITIES)[number];
export type FamilyStatus = 'eligible' | 'unsupported' | 'insufficient_human_evidence';

export interface Family { id: string; kind: 'operational' | 'ai_layer'; requires: Capability[]; humanEvidence?: boolean }
export const FAMILIES: Family[] = [
  { id: 'first_response_sla', kind: 'operational', requires: [] },
  { id: 'queue_wait', kind: 'operational', requires: [] },
  { id: 'language_starvation', kind: 'operational', requires: [] },
  { id: 'load_imbalance', kind: 'operational', requires: [] },
  { id: 'reassignment_churn', kind: 'operational', requires: [] },
  { id: 'close_reason_mix', kind: 'operational', requires: [] },
  { id: 'recontact_chains', kind: 'operational', requires: [] },
  { id: 'tool_failure', kind: 'ai_layer', requires: ['tool_call'] },
  { id: 'identity_friction', kind: 'ai_layer', requires: ['identity_check'] },
  { id: 'approval_bottleneck', kind: 'ai_layer', requires: ['approval'] },
  { id: 'routing_misroute', kind: 'ai_layer', requires: ['routing_step'] },
  { id: 'suggestion_quality', kind: 'ai_layer', requires: ['suggestion'] },
  { id: 'topic_drift', kind: 'ai_layer', requires: ['topic'] },
  { id: 'human_resolution_verified', kind: 'ai_layer', requires: ['case_close.resolved', 'csat'], humanEvidence: true },
];

export interface Counters {
  events_sent: number; unknown_event_type: number; denied_event_type: number; gap_suspected: number; late_event: number;
}
export interface Source {
  id: SourceId; provenance: 'stand_in'; profileVersion: string; phase: number | null;
  hasEventTimeline: boolean; teamGeneratedExcluded: number | null;
  capabilities: Record<Capability, boolean>; channels: string[]; counters: Counters | null;
}
const caps = (present: Capability[]) => Object.fromEntries(CAPABILITIES.map((c) => [c, present.includes(c)])) as Record<Capability, boolean>;
const ALL_RICH = CAPABILITIES.filter((c) => c !== 'teams');

export const DEMO_SOURCES: Source[] = [
  { id: 'e0_enriched', provenance: 'stand_in', profileVersion: 'platform_history 0.5.1', phase: null, hasEventTimeline: true, teamGeneratedExcluded: null, capabilities: caps(ALL_RICH), channels: ['app_chat', 'web_chat', 'phone', 'branch'], counters: null },
  { id: 'bank_original', provenance: 'stand_in', profileVersion: 'bank_dataset original', phase: null, hasEventTimeline: false, teamGeneratedExcluded: null, capabilities: caps(['complaint_id', 'topic', 'origin']), channels: ['phone', 'branch'], counters: null },
  {
    id: 'platform_live', provenance: 'stand_in', profileVersion: 'platform_live.phase1/1', phase: 1, hasEventTimeline: true, teamGeneratedExcluded: 412,
    capabilities: caps(['teams']), channels: ['app_chat', 'web_chat'],
    counters: { events_sent: 18423, unknown_event_type: 7, denied_event_type: 2, gap_suspected: 1, late_event: 5 },
  },
];

export interface Eligibility { family: string; kind: Family['kind']; status: FamilyStatus; missing: Capability[] }
export function eligibility(s: Source): Eligibility[] {
  return FAMILIES.map((f) => {
    // Operational families need an event timeline (a source fact, not an exporter capability): unsupported with no capability named.
    const miss: Capability[] = f.requires.filter((c) => !s.capabilities[c]);
    const noTimeline = f.kind === 'operational' && !s.hasEventTimeline;
    const status: FamilyStatus = miss.length === 0 && !noTimeline ? 'eligible' : f.humanEvidence ? 'insufficient_human_evidence' : 'unsupported';
    return { family: f.id, kind: f.kind, status, missing: miss };
  });
}

export interface Insight {
  id: string; family: string; status: 'waiting_dependency' | 'insufficient_human_evidence'; // spec 32.3 codes only
  // reason 'no_core_artifact_in_phase1' is PROPOSED vocabulary, pending Codex (not a confirmed code).
  reason: string; evidenceKind: 'observed'; denominator: number; provenance: string;
}
/** Phase 1 findings stop before the proposal step (spec 32.3): read-only insight, never a publish path. */
export function insightsOf(s: Source): Insight[] {
  if (s.id !== 'platform_live') return [];
  return [
    { id: 'first_response_sla', family: 'first_response_sla', status: 'waiting_dependency', reason: 'no_core_artifact_in_phase1', evidenceKind: 'observed', denominator: 3120, provenance: 'sla_due_at (declarado por la plataforma)' },
    { id: 'language_starvation', family: 'language_starvation', status: 'waiting_dependency', reason: 'no_core_artifact_in_phase1', evidenceKind: 'observed', denominator: 940, provenance: 'staff.languages' },
    { id: 'reassignment_churn', family: 'reassignment_churn', status: 'waiting_dependency', reason: 'no_core_artifact_in_phase1', evidenceKind: 'observed', denominator: 2210, provenance: 'assignments' },
  ];
}
