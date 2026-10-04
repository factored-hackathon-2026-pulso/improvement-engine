import { z } from 'zod';
// Read model of /internal/v1/automation (debug-api). Tolerant: unknown fields are dropped, unknown enums stay strings.
const str = z.string();
export const Metric = z.object({
  status: str, value: z.number().optional(), numerator: z.number().optional(), denominator: z.number().optional(),
  source: str.optional(), simulated: z.boolean().optional(), reason: str.optional(),
});
export const Measure = z.object({
  kind: str, numerator: z.number().optional(), denominator: z.number().optional(), source: str.optional(), simulated: z.boolean().optional(),
  handed: z.number().nullable().optional(),
});
export const Stage = z.union([z.number(), str]);
export const CaseType = z.object({
  type_id: str, label: str.nullable(), group: str.nullable(), stage: Stage, agent_proposed: z.boolean(), simulated: z.boolean(),
  blocked_by: str.nullable(), cases_today: z.number().nullable(), measure: Measure, metrics: z.record(Metric),
});
export const Banner = z.object({
  type_id: str, label: str.nullable(), proposal_id: str.nullable(), simulated: z.boolean(),
  numerator: z.number().nullable(), denominator: z.number().nullable(),
});
export const CaseTypeList = z.object({
  as_of: str, data_origin: str, revision: z.number(),
  doubles: z.array(z.object({ id: str, mode: str, scope: z.array(str) })),
  thresholds: z.record(z.unknown()), assumptions: z.array(str).default([]), banner: Banner.nullable(), case_types: z.array(CaseType),
});
export const ThresholdRow = z.object({ key: str, metric: str, min: z.number(), unit: str, today: Metric, met: z.boolean(), window: z.number().optional() });
export const Proposal = z.object({
  proposal_id: str, simulated: z.boolean(), state: str,
  target: z.object({ agent_id: str, alias: str, artifact: str, candidate_artifact: str.optional(), eval_suite: str.optional() }),
  related_agents: z.array(str).optional(), origin: str.optional(), evaluability: str.optional(),
  alternative_target: z.object({ agent_id: str, alias: str, artifact: str, evaluability: str.optional() }).optional(), candidate_hash: str, ledger_verdict: str, link_grade: str, run_id: str, change: str.optional(),
});
export const CaseTypeDetail = CaseType.extend({
  history: z.array(z.object({ stage: str, since: str.nullable() })),
  drafts_last_100: z.object({
    status: str, window: z.number().optional(), as_is: z.number().optional(), minor: z.number().optional(), discarded: z.number().optional(),
    source: str.optional(), simulated: z.boolean().optional(), reason: str.optional(),
  }),
  thresholds_today: z.array(ThresholdRow),
  proposal: Proposal.nullable(),
});
export const ActionResult = z.object({ state: str, simulated: z.boolean(), proposal_id: str });
export type Metric = z.infer<typeof Metric>;
export type CaseType = z.infer<typeof CaseType>;
export type CaseTypeList = z.infer<typeof CaseTypeList>;
export type CaseTypeDetail = z.infer<typeof CaseTypeDetail>;
export type Proposal = z.infer<typeof Proposal>;
export type ActionResult = z.infer<typeof ActionResult>;
