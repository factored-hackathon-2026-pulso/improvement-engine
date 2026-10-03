import { z } from 'zod';
// Hand-written from plan 16.10 (never from Rust). Strict on structure, tolerant on enums:
// an unknown enum value stays a string and is rendered as "unrecognised: <code>".
const str = z.string();

/**
 * Schemas for routes whose shape plan 16.10 does not define are provisional: they carry
 * `metadata.consumer_proposal=true` (zod description) and a tracking ref until Codex records the shape.
 */
export const CONSUMER_PROPOSALS: Record<string, string> = {
  Investigation: 'R5/M1', Gates: 'R5', Diff: 'R7/M2/M3', Memory: 'M2', Decision: 'R4',
  Session: 'CO-02/CLQ-22', Profile: 'M6/CLQ-28', StepUp: 'CO-02',
};
const proposal = <T extends z.ZodTypeAny>(name: string, schema: T): T =>
  schema.describe(JSON.stringify({ consumer_proposal: true, ref: CONSUMER_PROPOSALS[name] }));
export const proposalMeta = (schema: z.ZodTypeAny): { consumer_proposal: true; ref: string } | null => {
  try {
    const m = JSON.parse(schema.description ?? 'null') as { consumer_proposal?: boolean; ref?: string } | null;
    return m?.consumer_proposal === true && m.ref ? { consumer_proposal: true, ref: m.ref } : null;
  } catch { return null; }
};
export const EntityRef = z.object({ kind: str, id: str });
export const Envelope = z.object({
  schema_version: z.literal('1'), tenant_id: str, projection_revision: z.number().int().nonnegative(),
  status: str, blocking_reasons: z.array(str), available_commands: z.array(str),
});
export const GraphNode = z.object({
  node_id: str, label: str, stage: str, status: str, depends_on: z.array(str), reason_code: str.nullable(), node_kind: str,
  // 32 lowercase hex or null. Missing/invalid is treated as "no trace" (degraded trace panel), never trusted.
  trace_id: z.unknown().transform((v): string | null => (typeof v === 'string' && /^[0-9a-f]{32}$/.test(v) ? v : null)),
});
export const Graph = Envelope.extend({ nodes: z.array(GraphNode) });
export const RunList = Envelope.extend({
  items: z.array(z.object({ run_id: str, title: str, state: str, origin: str, projection_revision: z.number() })),
});
export const Evidence = z.object({
  evidence_ref: z.object({ id: str, digest: str, media_type: str }),
  relation: str, summary: str, source_kind: str, validation: str, available_at: str,
});
export const Investigation = proposal('Investigation', Envelope.extend({
  hypothesis: str.nullable(), verifier: str, evidence: z.array(Evidence),
}));
const GateStatus = z.object({ status: str, reason_code: str.nullable() });
export const Gates = proposal('Gates', Envelope.extend({
  native: GateStatus, improvement: GateStatus, combined: z.object({ decision: str, reason_code: str.nullable() }),
}));
export const Diff = proposal('Diff', Envelope.extend({ lines: z.array(z.object({ op: str, text: str })) }));
export const Memory = proposal('Memory', Envelope.extend({
  items: z.array(z.object({ memory_id: str, title: str, status: str, revoked: z.boolean() })),
}));
export const Decision = proposal('Decision', Envelope.extend({ needs_step_up: z.boolean(), domain_revision: z.number().int().nonnegative() }));
export const Session = proposal('Session', z.object({
  principal: str, tenant_id: str, scopes: z.array(str), expires_at: str, csrf_token: str,
  auth: z.object({ simulated: z.boolean(), level: str, auth_at: str }),
}));
export const Profile = proposal('Profile', z.object({ target: str, runtime_profile: str, doubles: z.array(str), pin: str.nullable() }));
export const Conflict = z.object({ expected_revision: z.number().int(), current_revision: z.number().int(), diff_ref: z.unknown().nullable() });
export const Problem = z.object({
  code: str, message: str, correlation_id: str, retryable: z.boolean(), conflict: Conflict.nullable().optional().catch(null),
});
export const Accepted = z.object({ command_ref: EntityRef, status_url: str });
export const CommandStatus = z.object({ state: str });
export const DebugEventSchema = z.object({
  event_id: str, run_id: str, sequence: z.number().int(), entity_ref: EntityRef,
  projection_revision: z.number().int(), kind: str,
});
export const StepUp = proposal('StepUp', z.object({ level: str }));
