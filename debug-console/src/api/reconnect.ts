// Pure helpers for the own SSE reader: backoff, close classification, frame parsing.
export const BACKOFF_BASE_MS = 500;
export const BACKOFF_CAP_MS = 15000;

/** Full-jitter exponential backoff. `random` is injected so tests need no timers. */
export const backoffDelay = (attempt: number, random: () => number = Math.random): number => {
  const ceiling = Math.min(BACKOFF_CAP_MS, BACKOFF_BASE_MS * 2 ** attempt);
  return Math.floor(ceiling * random());
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
