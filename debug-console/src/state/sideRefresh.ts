// SSE events that change data the run view reads outside the graph (the mode banner's profile and the Gates panel).
import type { DebugEvent } from './runStore';

export interface SideRefresh { profile: boolean; gates: boolean }

export const needsSideRefresh = (events: DebugEvent[]): SideRefresh => ({
  profile: events.some((e) => e.kind === 'doubles_declared'),
  gates: events.some((e) => e.kind === 'gates_set'),
});

export interface Debounced { (): void; cancel: () => void }

/** Trailing debounce: a burst of events costs one refetch, never a polling storm. */
export function createDebounced(fn: () => void, ms: number): Debounced {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const d = (() => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => { timer = null; fn(); }, ms);
  }) as Debounced;
  d.cancel = () => { if (timer) clearTimeout(timer); timer = null; };
  return d;
}

export const SIDE_REFRESH_DEBOUNCE_MS = 150;
