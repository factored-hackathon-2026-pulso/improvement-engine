// Pure helpers for the own SSE reader: backoff, close classification, frame parsing.
// Plan 16.13.4: backoff 1-30 s with jitter.
export const BACKOFF_MIN_MS = 1000;
export const BACKOFF_CAP_MS = 30000;

/** Exponential ceiling (1 s, 2 s, 4 s ... 30 s) with jitter drawn from [1 s, ceiling]. `random` is injected so tests need no timers. */
export const backoffDelay = (attempt: number, random: () => number = Math.random): number => {
  const ceiling = Math.min(BACKOFF_CAP_MS, BACKOFF_MIN_MS * 2 ** Math.min(attempt, 30));
  return BACKOFF_MIN_MS + Math.floor((ceiling - BACKOFF_MIN_MS) * random());
};

export type CloseAction = 'retry' | 'resnapshot' | 'session_expired' | 'forbidden';
/** 0 = transport cut. 403/404 are deliberately indistinguishable (no existence leak). */
export const classifyClose = (status: number): CloseAction => {
  if (status === 410) return 'resnapshot';
  if (status === 401) return 'session_expired';
  if (status === 403 || status === 404) return 'forbidden';
  return 'retry';
};

export interface SseFrame { id: string | null; data: string }
export function parseSseFrames(input: string): { frames: SseFrame[]; rest: string } {
  let buf = input.replace(/\r\n/g, '\n');
  const frames: SseFrame[] = [];
  for (let i = buf.indexOf('\n\n'); i >= 0; i = buf.indexOf('\n\n')) {
    const lines = buf.slice(0, i).split('\n');
    buf = buf.slice(i + 2);
    const data = lines.filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim()).join('');
    if (!data) continue;
    const idLine = lines.find((l) => l.startsWith('id:'));
    frames.push({ id: idLine ? idLine.slice(3).trim() : null, data });
  }
  return { frames, rest: buf };
}

/** "Silence is not completion": no signal (event or heartbeat comment) for more than 2x the heartbeat means stale. */
export const isStale = (lastSignalAt: number | null, now: number, heartbeatMs: number): boolean =>
  lastSignalAt !== null && now - lastSignalAt > 2 * heartbeatMs;

/** 410 `cursor_expired` body -> floor for the snapshot. An unusable cursor is null; events are never invented. */
export function parseGone(body: unknown): { recoveryCursor: number | null } {
  const v = typeof body === 'object' && body !== null ? (body as { recovery_after_sequence?: unknown }).recovery_after_sequence : undefined;
  return { recoveryCursor: typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : null };
}
