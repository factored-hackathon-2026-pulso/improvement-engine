// Pure stream reducer. SSE is a wake-up: events never mutate nodes; the caller re-reads /graph.
export interface DebugEvent {
  event_id: string; run_id: string; sequence: number;
  entity_ref: { kind: string; id: string }; projection_revision: number; kind: string;
}
export interface StreamState { lastSeq: number; pending: Record<number, DebugEvent> }
export type Effect =
  | { type: 'apply'; events: DebugEvent[] }
  | { type: 'duplicate' }
  | { type: 'catchup'; after_sequence: number };

export const initStream = (lastSeq: number): StreamState => ({ lastSeq, pending: {} });

export function applyEvent(state: StreamState, ev: DebugEvent): { state: StreamState; effect: Effect } {
  if (ev.sequence <= state.lastSeq) return { state, effect: { type: 'duplicate' } };
  const pending = { ...state.pending, [ev.sequence]: ev };
  if (state.lastSeq + 1 !== Math.min(...Object.keys(pending).map(Number))) {
    return { state: { ...state, pending }, effect: { type: 'catchup', after_sequence: state.lastSeq } };
  }
  const events: DebugEvent[] = [];
  let seq = state.lastSeq;
  for (let next = pending[seq + 1]; next; next = pending[seq + 1]) {
    events.push(next);
    delete pending[seq + 1];
    seq += 1;
  }
  return { state: { lastSeq: seq, pending }, effect: { type: 'apply', events } };
}

/** Drops projections older than what is already shown. */
export const acceptRevision = (current: number, incoming: number): boolean => incoming >= current;
