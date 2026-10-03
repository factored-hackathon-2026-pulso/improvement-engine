import { z } from 'zod';

// Hand-written from spec section 25 (never from Rust). Strict on structure, tolerant on enums:
// an unknown enum value stays a string. Field NAMES of the per-panel projections are provisional where
// the spec lists columns but not names (model-calls, queries, evals, memory-diff, external-commands,
// health): they are tracked in docs/journal-c-0001.md as gaps for the control-api owner.
const str = z.string();
const nullish = <T extends z.ZodTypeAny>(s: T) => s.nullable().optional().transform((v) => v ?? null);

/** 32 lowercase hex or null. Missing/invalid is "no trace" (trace_unavailable), never trusted. */
const traceId = z.unknown().transform((v): string | null => (typeof v === 'string' && /^[0-9a-f]{32}$/.test(v) ? v : null));

/** Common projection envelope: every debug projection carries `projection_revision` and `as_of` (spec 25). */
const envelopeShape = {
  schema_version: z.literal('1'), tenant_id: str, projection_revision: z.number().int().nonnegative(),
  // as_of is required by the spec; older fixtures omit it, so it is optional until the real API exists.
  as_of: str.optional(),
  status: str, blocking_reasons: z.array(str), available_commands: z.array(str),
};
const page = <T extends z.ZodTypeAny>(item: T) => z.object({ ...envelopeShape, items: z.array(item), next_cursor: z.string().nullable().optional().transform((v) => v ?? null) });

export const RunSummary = z.object({ run_id: str, title: str, state: str, origin: str, projection_revision: z.number().int() });
export const RunList = page(RunSummary);

export const GraphNode = z.object({
  node_id: str, label: str, stage: str, status: str, depends_on: z.array(str), reason_code: z.string().nullable(), node_kind: str, trace_id: traceId,
});
export const Graph = z.object({ ...envelopeShape, nodes: z.array(GraphNode) });

/** RunEvent per spec 25 "Contrato de instrumentacion". `sequence` is monotonic per run. */
export const RunEvent = z.object({
  sequence: z.number().int().nonnegative(), run_ref: str, job_ref: nullish(str), stage: str, event_code: str, status: str,
  reason_code: nullish(str), artifact_ref: nullish(str), trace_id: traceId.optional().transform((v) => v ?? null), details_ref: nullish(str),
  occurred_at: str,
});
export type RunEvent = z.infer<typeof RunEvent>;
export const EventPage = page(RunEvent);

export const ModelCall = z.object({
  call_id: str, run_ref: str, purpose: str, route: str, judgment_type: str, input_digest: str, output_digest: nullish(str),
  payload_redacted: z.boolean(), validation: str, tokens_in: z.number().int().nullable(), tokens_out: z.number().int().nullable(),
  usd: z.number().nullable(), latency_ms: z.number().nullable(), outcome: str, retry_of: nullish(str), error_code: nullish(str), decision_ref: nullish(str),
  trace_id: traceId.optional().transform((v) => v ?? null),
});
export const ModelCallPage = page(ModelCall);

export const QueryReceipt = z.object({
  query_id: str, run_ref: str,
  /** Sanitised SQL/AST or null; raw SQL with sensitive literals is never exposed (spec 25). */
  sql_sanitized: z.string().nullable(), sql_digest: str, param_classes: z.array(str),
  rows: z.number().int().nullable(), bytes: z.number().int().nullable(), duration_ms: z.number().nullable(), truncated: z.boolean(),
  outcome: str, quality_findings: z.array(str),
});
export const QueryPage = page(QueryReceipt);

const Gate = z.object({ status: str, reason_code: nullish(str) });
/** Two independent gates, never one green (spec 25 table, Evaluacion). `final` is only an aggregate. */
export const EvalCard = z.object({
  candidate_hash: str, suite: z.object({ id: str, version: str, digest: str }), base_staging: nullish(str), baseline_runtime: nullish(str),
  core_gate: Gate, improvement_gate: Gate, final: nullish(str),
});
export const EvalPage = page(EvalCard);

const DiffLine = z.object({ op: str, text: str });
export const MemoryDiff = z.object({
  ...envelopeShape,
  head_ref: nullish(str), pages_read: z.array(z.object({ page_ref: str, digest: str })), proposed: z.array(DiffLine), published: z.array(DiffLine),
  lint: z.array(str), cas_lost: z.boolean(), revocations: z.array(z.object({ page_ref: str, revoked_at: nullish(str) })),
});

/** "Requested" and "confirmed" are separate facts (spec 25): confirmed_at stays null until the platform confirms. */
export const ExternalCommand = z.object({
  command_ref: str, kind: str, state: str, requested_at: str, confirmed_at: nullish(str), observation_ref: nullish(str),
});
export const ExternalCommandPage = page(ExternalCommand);

export const DependencyHealth = z.object({ name: str, status: str, checked_at: nullish(str), reason_code: nullish(str) });
export const HealthPage = page(DependencyHealth);

export const Session = z.object({
  principal: str, tenant_id: str, scopes: z.array(str), expires_at: str, csrf_token: str,
  auth: z.object({ simulated: z.boolean(), level: str, auth_at: str }),
});
export const Mode = z.object({ target: str, runtime_profile: str, doubles: z.array(str) });

export const CommandAck = z.object({ command_ref: z.object({ kind: str, id: str }), status_url: str, state: str.default('requested') });

export const Conflict = z.object({ expected_revision: z.number().int(), current_revision: z.number().int(), diff_ref: z.unknown().nullable().optional() });
export interface Recovery { snapshot_ref: string; after_sequence: number }

/** The one error type every provider throws. `status` 0 = transport failure. */
export class DebugApiError extends Error {
  constructor(
    public status: number, public code: string, message: string, public correlationId: string, public retryable: boolean,
    public conflict: z.infer<typeof Conflict> | null = null, public recovery: Recovery | null = null,
  ) { super(message); this.name = 'DebugApiError'; }
}

const ProblemBody = z.object({
  code: str, message: str.optional(), correlation_id: str.optional(), retryable: z.boolean().optional(),
  conflict: Conflict.nullable().optional().catch(null),
  recovery: z.object({ snapshot_ref: str, after_sequence: z.number().int().nonnegative() }).optional().catch(undefined),
  // legacy fixture-server shape (CLQ-24 draft)
  recovery_after_sequence: z.number().int().nonnegative().optional().catch(undefined), snapshot_url: str.optional().catch(undefined),
}).passthrough();
const RETRYABLE = new Set([429, 502, 503, 504]);

/** Uniform error envelope. The body is never echoed into the message: unrecognised bodies become `unrecognised_error`. */
export function parseProblem(status: number, body: unknown): DebugApiError {
  const p = ProblemBody.safeParse(body);
  if (!p.success) return new DebugApiError(status, 'unrecognised_error', 'unrecognised_error', '', status >= 500 || RETRYABLE.has(status));
  const d = p.data;
  const recovery = d.recovery ?? (d.snapshot_url !== undefined && d.recovery_after_sequence !== undefined ? { snapshot_ref: d.snapshot_url, after_sequence: d.recovery_after_sequence } : null);
  return new DebugApiError(status, d.code, d.message ?? d.code, d.correlation_id ?? '', d.retryable ?? RETRYABLE.has(status), d.conflict ?? null, recovery);
}
