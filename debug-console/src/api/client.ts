import { z } from 'zod';
import * as S from './schemas';
import type { DebugEvent } from '../state/runStore';
import { scrubAndReport } from '../security/clientScrubber';
import { backoffDelay, classifyClose, parseGone, parseSseFrames, type CloseAction } from './reconnect';

export const DEBUG = '/internal/v1/debug';
export type Conflict = z.infer<typeof S.Conflict>;
export class ApiError extends Error {
  constructor(public status: number, public code: string, public conflict: Conflict | null = null) { super(code); }
}
let csrf = '';
export const setCsrf = (t: string) => { csrf = t; };

const expiredListeners = new Set<() => void>();
/** Any 401 (plain request or SSE) means the session is gone: subscribers flip the UI to session-expired. */
export const onSessionExpired = (fn: () => void): (() => void) => {
  expiredListeners.add(fn);
  return () => { expiredListeners.delete(fn); };
};
const sessionGone = () => { csrf = ''; expiredListeners.forEach((fn) => fn()); };
/** Used by the DebugApi providers so a 401 from the port flips the same session-expired state. */
export const reportSessionExpired = sessionGone;

export type DataProvider = 'http' | 'fixture' | 'stand-in';
export interface ClientConfig { provider: string; sseHeartbeatMs: number; dataProvider: DataProvider; apiBase: string }
/** public/config.json is read at runtime; absent or invalid values fall back to safe defaults (provider "unknown"). */
export async function loadConfig(): Promise<ClientConfig> {
  try {
    const c = (await (await fetch('/config.json', { cache: 'no-store' })).json()) as { provider?: unknown; sseHeartbeatMs?: unknown; dataProvider?: unknown; apiBase?: unknown };
    const hb = typeof c.sseHeartbeatMs === 'number' && c.sseHeartbeatMs >= 50 ? c.sseHeartbeatMs : 5000;
    // dataProvider selects the DebugApi provider (http | fixture | stand-in); anything else falls back to http, the real transport.
    const dataProvider: DataProvider = c.dataProvider === 'fixture' || c.dataProvider === 'stand-in' ? c.dataProvider : 'http';
    const apiBase = typeof c.apiBase === 'string' && /^(https?:\/\/[^/]+)?$/.test(c.apiBase) ? c.apiBase : '';
    return { provider: typeof c.provider === 'string' ? c.provider : 'unknown', sseHeartbeatMs: hb, dataProvider, apiBase };
  } catch { return { provider: 'unknown', sseHeartbeatMs: 5000, dataProvider: 'http', apiBase: '' }; }
}

async function request<T>(schema: z.ZodType<T, z.ZodTypeDef, unknown>, path: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try { res = await fetch(path, { credentials: 'same-origin', cache: 'no-store', ...init }); } catch { throw new ApiError(0, 'network_error'); }
  const body: unknown = await res.json().catch(() => ({}));
  if (!res.ok) {
    if (res.status === 401) sessionGone();
    const p = S.Problem.safeParse(body);
    throw new ApiError(res.status, p.success ? p.data.code : 'unrecognised_error', p.success ? (p.data.conflict ?? null) : null);
  }
  return schema.parse(scrubAndReport(body, path.split('?')[0] ?? path));
}
const post = <T,>(schema: z.ZodType<T, z.ZodTypeDef, unknown>, path: string, body: unknown, key?: string) => {
  if (!csrf) return Promise.reject(new ApiError(0, 'csrf_missing')); // never send a mutation with an empty token
  return request(schema, path, {
    method: 'POST', body: JSON.stringify(body),
    headers: { 'content-type': 'application/json', 'x-csrf-token': csrf, ...(key ? { 'idempotency-key': key } : {}) },
  });
};

export const api = {
  session: () => request(S.Session, '/api/v1/auth/session'),
  stepUp: () => post(S.StepUp, '/api/v1/auth/step-up', {}),
  profile: () => request(S.Profile, `${DEBUG}/profile`),
  runs: () => request(S.RunList, `${DEBUG}/runs`),
  graph: (id: string) => request(S.Graph, `${DEBUG}/runs/${id}/graph`),
  events: (id: string, after: number) =>
    request(z.object({ items: z.array(S.DebugEventSchema) }), `${DEBUG}/runs/${id}/events?after_sequence=${after}`),
  investigation: (id: string) => request(S.Investigation, `${DEBUG}/runs/${id}/investigation`),
  gates: (id: string) => request(S.Gates, `${DEBUG}/runs/${id}/gates`),
  diff: (proposalId: string) => request(S.Diff, `${DEBUG}/proposals/${encodeURIComponent(proposalId)}/diff`),
  alternatives: (id: string) => request(S.Alternatives, `${DEBUG}/runs/${id}/alternatives`),
  memory: () => request(S.Memory, `${DEBUG}/memory`),
  runDecision: (id: string) => request(S.RunDecision, `${DEBUG}/runs/${id}/decision`),
  decision: () => request(S.Decision, `${DEBUG}/decisions/dec-1`),
  respond: (note: string, key: string, expectedRevision: number) =>
    post(S.Accepted, `${DEBUG}/decisions/dec-1/responses`, { expected_revision: expectedRevision, response: 'approve', note }, key),
  command: (path: string) => request(S.CommandStatus, path),
};

export interface StreamHandlers {
  onEvent: (e: DebugEvent) => void;
  /** Called every time a connection is established; `reconnect` is true after a drop. */
  onOpen: (reconnect: boolean) => void;
  /** Transport dropped; a retry is scheduled. */
  onDrop: (attempt: number) => void;
  /** 410: the cursor was purged; caller must re-read the snapshot and use `recoveryCursor` as the floor (a new connection follows). */
  onResnapshot: (info: { recoveryCursor: number | null }) => void;
  /** Any bytes received (events or comment-only heartbeats): the stream is alive. */
  onActivity?: () => void;
  /** Terminal: 401 / 403 / 404. No retry. */
  onFatal: (action: Exclude<CloseAction, 'retry' | 'resnapshot'>) => void;
}
export interface StreamOptions {
  lastEventId?: () => number;
  random?: () => number;
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
}
const defaultSleep = (ms: number, signal: AbortSignal) => new Promise<void>((ok) => {
  const t = setTimeout(ok, ms);
  signal.addEventListener('abort', () => { clearTimeout(t); ok(); }, { once: true });
});

/**
 * Own SSE reader over fetch (native EventSource hides HTTP status). Reconnects with jittered
 * backoff and `Last-Event-ID`; 410 asks for a fresh snapshot; 401/403/404 are terminal. Returns a stop function.
 */
export function streamEvents(runId: string, h: StreamHandlers, opts: StreamOptions = {}): () => void {
  const ctl = new AbortController();
  const sleep = opts.sleep ?? defaultSleep;
  void (async () => {
    let attempt = 0;
    let opened = false;
    let gone = 0; // consecutive 410s: the second one backs off instead of hammering
    while (!ctl.signal.aborted) {
      let status = 0;
      let recoveryCursor: number | null = null;
      try {
        const last = opts.lastEventId?.() ?? 0;
        const res = await fetch(`${DEBUG}/runs/${runId}/events/stream`, {
          signal: ctl.signal, credentials: 'same-origin', cache: 'no-store', headers: last > 0 ? { 'last-event-id': String(last) } : {},
        });
        if (!res.ok || !res.body) {
          status = res.status;
          if (status === 410) recoveryCursor = parseGone(await res.json().catch(() => null)).recoveryCursor;
        } else {
          h.onOpen(opened); opened = true; attempt = 0; gone = 0; h.onActivity?.();
          const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
          let buf = '';
          for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
            h.onActivity?.();
            const r = parseSseFrames(buf + value);
            buf = r.rest;
            for (const f of r.frames) {
              let json: unknown;
              try { json = JSON.parse(f.data); } catch { continue; }
              const parsed = S.DebugEventSchema.safeParse(scrubAndReport(json, 'sse'));
              if (parsed.success) h.onEvent(parsed.data);
            }
          }
        }
      } catch {
        if (ctl.signal.aborted) return;
      }
      if (ctl.signal.aborted) return;
      const action = classifyClose(status);
      if (action === 'session_expired') sessionGone();
      if (action === 'session_expired' || action === 'forbidden') return h.onFatal(action);
      if (action === 'resnapshot') {
        opened = false; h.onResnapshot({ recoveryCursor }); gone += 1;
        if (gone === 1) continue;
      }
      h.onDrop(attempt);
      await sleep(backoffDelay(attempt, opts.random), ctl.signal);
      attempt += 1;
    }
  })();
  return () => ctl.abort();
}
