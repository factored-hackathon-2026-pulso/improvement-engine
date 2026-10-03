import { z } from 'zod';
// Hand-written from plan 16.10 (never from Rust). Strict on structure, tolerant on enums:
// an unknown enum value stays a string and is rendered as "unrecognised: <code>".
const str = z.string();
export const EntityRef = z.object({ kind: str, id: str });
export const Envelope = z.object({
  schema_version: z.literal('1'), tenant_id: str, projection_revision: z.number().int().nonnegative(),
  status: str, blocking_reasons: z.array(str), available_commands: z.array(str),
});
export const GraphNode = z.object({
  node_id: str, label: str, stage: str, status: str, depends_on: z.array(str), reason_code: str.nullable(), node_kind: str,
});
export const Graph = Envelope.extend({ nodes: z.array(GraphNode) });
export const RunList = Envelope.extend({
  items: z.array(z.object({ run_id: str, title: str, state: str, origin: str, projection_revision: z.number() })),
});
export const Evidence = z.object({
  evidence_ref: z.object({ id: str, digest: str, media_type: str }),
  relation: str, summary: str, source_kind: str, validation: str, available_at: str,
});
export const Investigation = Envelope.extend({
  hypothesis: str.nullable(), verifier: str, evidence: z.array(Evidence),
});
const GateStatus = z.object({ status: str, reason_code: str.nullable() });
export const Gates = Envelope.extend({
  native: GateStatus, improvement: GateStatus, combined: z.object({ decision: str, reason_code: str.nullable() }),
});
export const Diff = Envelope.extend({ lines: z.array(z.object({ op: str, text: str })) });
export const Memory = Envelope.extend({
  items: z.array(z.object({ memory_id: str, title: str, status: str, revoked: z.boolean() })),
});
export const Decision = Envelope.extend({ needs_step_up: z.boolean() });
export const Session = z.object({
  principal: str, tenant_id: str, scopes: z.array(str), expires_at: str, csrf_token: str,
  auth: z.object({ simulated: z.boolean(), level: str, auth_at: str }),
});
export const Profile = z.object({ target: str, runtime_profile: str, doubles: z.array(str), pin: str.nullable() });
export const Problem = z.object({ code: str, message: str, correlation_id: str, retryable: z.boolean() });
export const Accepted = z.object({ command_ref: EntityRef, status_url: str });
export const CommandStatus = z.object({ state: str });
export const DebugEventSchema = z.object({
  event_id: str, run_id: str, sequence: z.number().int(), entity_ref: EntityRef,
  projection_revision: z.number().int(), kind: str,
});
