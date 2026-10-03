import { z } from 'zod';
import * as S from './schemas';
import type { DebugEvent } from '../state/runStore';

export const DEBUG = '/internal/v1/debug';
export class ApiError extends Error {
  constructor(public status: number, public code: string) { super(code); }
}
let csrf = '';
export const setCsrf = (t: string) => { csrf = t; };

async function request<T>(schema: z.ZodType<T>, path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, { credentials: 'same-origin', ...init });
  const body: unknown = await res.json().catch(() => ({}));
  if (!res.ok) {
    const p = S.Problem.safeParse(body);
    throw new ApiError(res.status, p.success ? p.data.code : 'unrecognised_error');
  }
  return schema.parse(body);
}
const post = <T,>(schema: z.ZodType<T>, path: string, body: unknown, key?: string) =>
  request(schema, path, {
    method: 'POST', body: JSON.stringify(body),
    headers: { 'content-type': 'application/json', 'x-csrf-token': csrf, ...(key ? { 'idempotency-key': key } : {}) },
  });

export const api = {
  session: () => request(S.Session, '/api/v1/auth/session'),
  stepUp: () => post(z.object({ level: z.string() }), '/api/v1/auth/step-up', {}),
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

/** Own SSE reader over fetch (native EventSource hides HTTP status). Returns a stop function. */
export function streamEvents(
  runId: string, onEvent: (e: DebugEvent) => void, onClose: (status: number) => void, onOpen: () => void = () => {},
): () => void {
  const ctl = new AbortController();
  void (async () => {
    try {
      const res = await fetch(`${DEBUG}/runs/${runId}/events/stream`, { signal: ctl.signal, credentials: 'same-origin' });
      if (!res.ok || !res.body) return onClose(res.status);
      onOpen();
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
      let buf = '';
      for (;;) {
        const { value, done } = await reader.read();
        if (done) return onClose(0);
        buf += value;
        let i = buf.indexOf('\n\n');
        while (i >= 0) {
          const frame = buf.slice(0, i);
          buf = buf.slice(i + 2);
          i = buf.indexOf('\n\n');
          const data = frame.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim()).join('');
          if (!data) continue;
          const parsed = S.DebugEventSchema.safeParse(JSON.parse(data));
          if (parsed.success) onEvent(parsed.data);
        }
      }
    } catch {
      if (!ctl.signal.aborted) onClose(0);
    }
  })();
  return () => ctl.abort();
}
