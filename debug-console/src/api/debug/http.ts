import type { z } from 'zod';
import { scrubAndReport } from '../../security/clientScrubber';
import * as D from './dto';
import { commandPath, type DebugCommand } from './commands';
import { runStream } from './stream';
import type { DebugApi, DebugSession, ModeDeclaration, ProviderKind, RunsQuery, StreamHandlers, StreamOptions } from './port';

export const DEBUG_PREFIX = '/internal/v1/debug';
export type FetchLike = (url: string, init?: RequestInit) => Promise<Response>;

export interface HttpProviderConfig {
  kind: ProviderKind;
  /** Origin (no trailing slash); empty = same-origin. */
  baseUrl?: string;
  /** Defaults to the global fetch, resolved at call time. In-process providers pass the backend's fetch. */
  fetch?: FetchLike;
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
  onSessionExpired?: () => void;
}

const defaultSleep = (ms: number, signal: AbortSignal) => new Promise<void>((ok) => {
  const t = setTimeout(ok, ms);
  signal.addEventListener('abort', () => { clearTimeout(t); ok(); }, { once: true });
});
const MAX_PAGE = 100; // spec 25: cursor page max 100

/** One provider implementation over `fetch`: `http` (real control-api), `fixture` and `stand-in` (in-process backend) differ only by config. */
export function createHttpProvider(cfg: HttpProviderConfig): DebugApi {
  const origin = cfg.baseUrl ?? '';
  const base = `${origin}${DEBUG_PREFIX}`;
  const doFetch: FetchLike = cfg.fetch ?? ((url, init) => globalThis.fetch(url, init));
  const sleep = cfg.sleep ?? defaultSleep;
  let csrf = '';

  async function send(path: string, init?: RequestInit): Promise<unknown> {
    let res: Response;
    try { res = await doFetch(path, { credentials: 'same-origin', cache: 'no-store', ...init }); } catch { throw new D.DebugApiError(0, 'network_error', 'network_error', '', true); }
    const body: unknown = await res.json().catch(() => ({}));
    if (!res.ok) {
      if (res.status === 401) { csrf = ''; cfg.onSessionExpired?.(); }
      throw D.parseProblem(res.status, body);
    }
    return scrubAndReport(body, path.split('?')[0] ?? path);
  }
  async function get<S extends z.ZodTypeAny>(schema: S, path: string): Promise<z.infer<S>> {
    const body = await send(path);
    const p = schema.safeParse(body);
    if (!p.success) throw new D.DebugApiError(200, 'invalid_response', 'invalid_response', '', false);
    return p.data as z.infer<S>;
  }
  const run = (id: string, tail: string) => `${base}/runs/${encodeURIComponent(id)}/${tail}`;

  const api: DebugApi = {
    provider: cfg.kind,
    async mode(): Promise<ModeDeclaration> {
      const m = await get(D.Mode, `${base}/profile`);
      return { provider: cfg.kind, target: m.target, runtime_profile: m.runtime_profile, doubles: m.doubles };
    },
    async session(): Promise<DebugSession> {
      const s = await get(D.Session, `${origin}/api/v1/auth/session`);
      csrf = s.csrf_token;
      return s;
    },
    runs(q: RunsQuery = {}) {
      const sp = new URLSearchParams();
      for (const [k, v] of Object.entries(q)) if (v !== undefined && k !== 'limit') sp.set(k, String(v));
      if (q.limit !== undefined) sp.set('limit', String(Math.max(1, Math.min(MAX_PAGE, Math.trunc(q.limit)))));
      const qs = sp.toString();
      return get(D.RunList, `${base}/runs${qs ? `?${qs}` : ''}`);
    },
    graph: (id) => get(D.Graph, run(id, 'graph')),
    events: (id, after) => get(D.EventPage, run(id, `events?after_sequence=${Math.max(0, Math.trunc(after))}`)),
    modelCalls: (id) => get(D.ModelCallPage, run(id, 'model-calls')),
    queries: (id) => get(D.QueryPage, run(id, 'queries')),
    evals: (id) => get(D.EvalPage, run(id, 'evals')),
    memoryDiff: (id) => get(D.MemoryDiff, run(id, 'memory-diff')),
    externalCommands: (id) => get(D.ExternalCommandPage, run(id, 'external-commands')),
    dependencyHealth: () => get(D.HealthPage, `${base}/health/dependencies`),
    openStream(runId: string, h: StreamHandlers, o: StreamOptions) {
      return runStream({
        fetchImpl: doFetch, base, runId, loadGraph: api.graph, loadEvents: api.events, sleep,
        onUnauthorized: () => { csrf = ''; cfg.onSessionExpired?.(); },
      }, h, o);
    },
    async command(c: DebugCommand) {
      if (!csrf) await api.session(); // never send a mutation without a CSRF token
      const body = await send(commandPath(c, base), {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'x-csrf-token': csrf, 'idempotency-key': c.idempotencyKey },
        body: JSON.stringify({ expected_revision: c.expectedRevision, reason: c.reason }),
      });
      const p = D.CommandAck.safeParse(body);
      if (!p.success) throw new D.DebugApiError(200, 'invalid_response', 'invalid_response', '', false);
      return p.data;
    },
  };
  return api;
}
