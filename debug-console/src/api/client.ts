import { z } from 'zod';
import * as S from './schemas';
import type { DebugEvent } from '../state/runStore';
import { scrubAndReport } from '../security/clientScrubber';
import { backoffDelay, classifyClose, parseSseFrames, type CloseAction } from './reconnect';

export const DEBUG = '/internal/v1/debug';
export class ApiError extends Error {
  constructor(public status: number, public code: string) { super(code); }
}
let csrf = '';
export const setCsrf = (t: string) => { csrf = t; };

async function request<T>(schema: z.ZodType<T>, path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, { credentials: 'same-origin', cache: 'no-store', ...init });
  const body: unknown = await res.json().catch(() => ({}));
  if (!res.ok) {
    const p = S.Problem.safeParse(body);
    throw new ApiError(res.status, p.success ? p.data.code : 'unrecognised_error');
  }
  return schema.parse(scrubAndReport(body, path.split('?')[0] ?? path));
}
const post = <T,>(schema: z.ZodType<T>, path: string, body: unknown, key?: string) =>
  request(schema, path, {
    method: 'POST', body: JSON.stringify(body),
    headers: { 'content-type': 'application/json', 'x-csrf-token': csrf, ...(key ? { 'idempotency-key': key } : {}) },
  });

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
  diff: () => request(S.Diff, `${DEBUG}/proposals/prop-1/diff`),
  memory: () => request(S.Memory, `${DEBUG}/memory`),
  decision: () => request(S.Decision, `${DEBUG}/decisions/dec-1`),
  respond: (note: string, key: string) =>
    post(S.Accepted, `${DEBUG}/decisions/dec-1/responses`, { response: 'approve', note }, key),
  command: (path: string) => request(S.CommandStatus, path),
};

export interface StreamHandlers {
  onEvent: (e: DebugEvent) => void;
  /** Called every time a connection is established; `reconnect` is true after a drop. */
  onOpen: (reconnect: boolean) => void;
  /** Transport dropped; a retry is scheduled. */
  onDrop: (attempt: number) => void;
  /** 410: the cursor was purged; caller must re-read the snapshot (a new connection follows). */
  onResnapshot: () => void;
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
      try {
        const last = opts.lastEventId?.() ?? 0;
        const res = await fetch(`${DEBUG}/runs/${runId}/events/stream`, {
          signal: ctl.signal, credentials: 'same-origin', cache: 'no-store', headers: last > 0 ? { 'last-event-id': String(last) } : {},
        });
        if (!res.ok || !res.body) {
          status = res.status;
        } else {
          h.onOpen(opened); opened = true; attempt = 0; gone = 0;
          const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
          let buf = '';
          for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
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
      if (action === 'session_expired' || action === 'forbidden') return h.onFatal(action);
      if (action === 'resnapshot') {
        opened = false; h.onResnapshot(); gone += 1;
        if (gone === 1) continue;
      }
      h.onDrop(attempt);
      await sleep(backoffDelay(attempt, opts.random), ctl.signal);
      attempt += 1;
    }
  })();
  return () => ctl.abort();
}
