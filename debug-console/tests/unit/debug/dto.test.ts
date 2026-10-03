import { describe, expect, it } from 'vitest';
import { commandState, SCOPE_FOR_COMMAND } from '../../../src/api/debug/commands';
import { DebugApiError, ModelCall, parseProblem, QueryReceipt, RunEvent } from '../../../src/api/debug/dto';

const ev = { sequence: 3, run_ref: 'r1', stage: 'scan', event_code: 'job_completed', status: 'ok', occurred_at: '2026-01-01T00:00:00Z' };

describe('RunEvent (spec 25 instrumentation contract)', () => {
  it('accepts the minimal event and the optional refs', () => {
    expect(RunEvent.parse(ev).sequence).toBe(3);
    expect(RunEvent.parse({ ...ev, job_ref: 'j', reason_code: 'x', artifact_ref: 'a', details_ref: 'd', trace_id: 'a'.repeat(32) }).trace_id).toBe('a'.repeat(32));
  });
  it('treats an invalid trace id as no trace (degraded), never trusting it', () => {
    expect(RunEvent.parse({ ...ev, trace_id: 'not-hex' }).trace_id).toBeNull();
  });
  it('rejects a missing/negative sequence and unknown extra enum is kept as string', () => {
    expect(RunEvent.safeParse({ ...ev, sequence: -1 }).success).toBe(false);
    expect(RunEvent.safeParse({ ...ev, sequence: undefined }).success).toBe(false);
    expect(RunEvent.parse({ ...ev, event_code: 'brand_new_code' }).event_code).toBe('brand_new_code');
  });
});

describe('treated payloads', () => {
  it('model calls carry digests and a payload_redacted flag, never raw prompts', () => {
    const m = ModelCall.parse({
      call_id: 'c1', run_ref: 'r1', purpose: 'investigate', route: 'profile-a', judgment_type: 'schema:x', input_digest: 'sha256:a', output_digest: 'sha256:b',
      payload_redacted: true, validation: 'valid', tokens_in: 1, tokens_out: 2, usd: null, latency_ms: 10, outcome: 'ok', error_code: null, decision_ref: null,
    });
    expect(m.payload_redacted).toBe(true);
    expect(Object.keys(m)).not.toContain('prompt');
  });
  it('query receipts show sanitized SQL or a digest, never raw SQL', () => {
    const q = QueryReceipt.parse({
      query_id: 'q1', run_ref: 'r1', sql_sanitized: null, sql_digest: 'sha256:c', param_classes: ['int'], rows: 1, bytes: 2, duration_ms: 3, truncated: false,
      outcome: 'denied', quality_findings: [],
    });
    expect(q.sql_sanitized).toBeNull();
    expect('sql' in q).toBe(false);
  });
});

describe('Problem / DebugApiError', () => {
  const base = { code: 'cursor_expired', message: 'm', correlation_id: 'c-1', retryable: false };
  it('normalises the 410 recovery (snapshot ref + cursor), including the legacy fixture shape', () => {
    expect(parseProblem(410, { ...base, recovery: { snapshot_ref: 's', after_sequence: 7 } }).recovery).toEqual({ snapshot_ref: 's', after_sequence: 7 });
    expect(parseProblem(410, { ...base, recovery_after_sequence: 7, snapshot_url: 'u' }).recovery).toEqual({ snapshot_ref: 'u', after_sequence: 7 });
  });
  it('an unrecognised error body still yields a uniform error without leaking the body', () => {
    const e = parseProblem(500, { secret: 'x' });
    expect(e).toBeInstanceOf(DebugApiError);
    expect([e.status, e.code, e.retryable]).toEqual([500, 'unrecognised_error', true]);
    expect(e.message).not.toContain('secret');
  });
  it('429/502/503/504 are retryable by default, 4xx are not', () => {
    for (const s of [429, 502, 503, 504]) expect(parseProblem(s, {}).retryable).toBe(true);
    expect(parseProblem(403, {}).retryable).toBe(false);
  });
});

describe('commandState: authority comes from the server, never from a role label', () => {
  const scopes = Object.values(SCOPE_FOR_COMMAND);
  it('enabled only when the server lists the command AND the session scope allows it', () => {
    expect(commandState('pause', ['pause'], scopes)).toEqual({ enabled: true });
    expect(commandState('pause', [], scopes)).toEqual({ enabled: false, reason: 'not_offered' });
    expect(commandState('pause', ['pause'], [])).toEqual({ enabled: false, reason: 'scope_missing' });
  });
  it('a role-looking scope or principal never enables anything', () => {
    expect(commandState('cancel', [], ['debug_operator', 'approver'])).toEqual({ enabled: false, reason: 'not_offered' });
    expect(commandState('cancel', ['cancel'], ['debug_operator'])).toEqual({ enabled: false, reason: 'scope_missing' });
  });
  it('an unknown command name is never enabled', () => {
    expect(commandState('promote_anyway', ['promote_anyway'], scopes)).toEqual({ enabled: false, reason: 'unknown_command' });
  });
  it('covers exactly the five spec commands', () => {
    expect(Object.keys(SCOPE_FOR_COMMAND).sort()).toEqual(['cancel', 'fork_replay', 'pause', 'resume', 'retry']);
  });
});
