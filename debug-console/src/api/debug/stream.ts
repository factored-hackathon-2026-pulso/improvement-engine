import { backoffDelay, classifyClose, parseSseFrames } from '../reconnect';
import { scrubAndReport } from '../../security/clientScrubber';
import { DebugApiError, parseProblem, RunEvent, type EventPage } from './dto';
import type { Graph, StreamHandlers, StreamOptions } from './port';
import type { z } from 'zod';

export interface StreamDeps {
  fetchImpl: (url: string, init?: RequestInit) => Promise<Response>;
  /** Base of the debug API, e.g. `${origin}/internal/v1/debug`. */
  base: string;
  runId: string;
  loadGraph: (runId: string) => Promise<Graph>;
  loadEvents: (runId: string, after: number) => Promise<z.infer<typeof EventPage>>;
  sleep: (ms: number, signal: AbortSignal) => Promise<void>;
  onUnauthorized: () => void;
}

/** Upper bound of one SSE frame (and of the unterminated tail): a hostile or broken server cannot grow the buffer without bound. */
const MAX_FRAME_CHARS = 1024 * 1024;

/** A snapshot ref must stay inside this run: a cursor/ref of another run is never followed (no cross-run leak). */
const refBelongsToRun = (ref: string, runId: string): boolean => !ref.includes('/') || ref.includes(`/runs/${runId}/`) || ref.endsWith(`/runs/${runId}`);

/**
 * Own SSE reader over fetch (EventSource hides the HTTP status). Contract (spec 25 / 29):
 * - resume with `after_sequence` and the `Last-Event-ID` header (= last applied sequence);
 * - dedup by (run, sequence); events of other runs are dropped; a gap is backfilled from GET /events, never guessed;
 * - 410 cursor_expired: read the snapshot, replace the projection (`onReset`), resume after the recovery cursor, invent nothing;
 * - 401/403/404 are terminal; any other close backs off with jitter and resumes. Silence is never completion.
 */
export function runStream(deps: StreamDeps, h: StreamHandlers, opts: StreamOptions): () => void {
  const ctl = new AbortController();
  const { runId, base } = deps;
  void (async () => {
    let last = opts.afterSequence;
    let attempt = 0;
    let opened = false;
    let gone = 0;
    const apply = (e: RunEvent) => { if (ctl.signal.aborted) return; last = e.sequence; h.onEvent(e); };
    const deliver = async (ev: RunEvent) => {
      if (ev.run_ref !== runId || ev.sequence <= last) return; // other run / duplicate
      if (ev.sequence > last + 1) {
        for (;;) { // gap: repage from the durable log
          const before = last;
          const page = await deps.loadEvents(runId, last);
          for (const e of [...page.items].sort((a, b) => a.sequence - b.sequence)) if (e.run_ref === runId && e.sequence === last + 1) apply(e);
          if (last === before) break;
        }
        if (ev.sequence <= last) return; // covered by the backfill
        if (ev.sequence !== last + 1) throw new DebugApiError(0, 'gap_unresolved', 'gap_unresolved', '', true); // reconnect and resume from `last`
      }
      apply(ev);
    };
    while (!ctl.signal.aborted) {
      let status = 0;
      let recovery: DebugApiError['recovery'] = null;
      try {
        const res = await deps.fetchImpl(`${base}/runs/${encodeURIComponent(runId)}/stream?after_sequence=${last}`, {
          signal: ctl.signal, credentials: 'same-origin', cache: 'no-store', headers: last > 0 ? { 'last-event-id': String(last) } : {},
        });
        if (!res.ok || !res.body) {
          status = res.status;
          if (status === 410) recovery = parseProblem(410, await res.json().catch(() => null)).recovery;
        } else {
          h.onOpen(opened); opened = true; attempt = 0; gone = 0; h.onActivity?.();
          const reader = res.body.getReader();
          const dec = new TextDecoder();
          let buf = '';
          for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
            h.onActivity?.();
            const r = parseSseFrames(buf + dec.decode(value, { stream: true }));
            buf = r.rest;
            if (buf.length > MAX_FRAME_CHARS) throw new DebugApiError(0, 'frame_too_large', 'frame_too_large', '', true); // unterminated frame: drop and resume from `last`
            for (const f of r.frames) {
              if (f.data.length > MAX_FRAME_CHARS) continue;
              let json: unknown;
              try { json = JSON.parse(f.data); } catch { continue; }
              const parsed = RunEvent.safeParse(scrubAndReport(json, 'sse'));
              if (parsed.success) await deliver(parsed.data);
            }
          }
        }
      } catch (e) {
        if (ctl.signal.aborted) return;
        if (e instanceof DebugApiError) status = e.status;
      }
      if (ctl.signal.aborted) return;
      const action = classifyClose(status);
      if (action === 'session_expired') deps.onUnauthorized();
      if (action === 'session_expired' || action === 'forbidden') return h.onFatal(action);
      if (action === 'resnapshot') {
        opened = false;
        if (recovery) {
          if (!refBelongsToRun(recovery.snapshot_ref, runId)) return h.onFatal('forbidden');
          try {
            const snapshot = await deps.loadGraph(runId);
            last = recovery.after_sequence;
            h.onReset({ snapshot, floor: recovery.after_sequence, snapshotRef: recovery.snapshot_ref });
            gone += 1;
            if (gone === 1) continue; // a second consecutive 410 backs off instead of hammering
          } catch (e) {
            if (ctl.signal.aborted) return;
            if (e instanceof DebugApiError && (e.status === 401 || e.status === 403 || e.status === 404)) {
              if (e.status === 401) deps.onUnauthorized();
              return h.onFatal(e.status === 401 ? 'session_expired' : 'forbidden');
            }
          }
        }
      }
      h.onDrop(attempt);
      await deps.sleep(backoffDelay(attempt, opts.random), ctl.signal);
      attempt += 1;
    }
  })();
  return () => ctl.abort();
}
