import { describe, it, expect } from 'vitest';
import { initStream, applyEvent, acceptRevision, type DebugEvent } from '../../src/state/runStore';

const ev = (sequence: number, node = 'verify'): DebugEvent => ({
  event_id: `e${sequence}`, run_id: 'r', sequence, entity_ref: { kind: 'node', id: node },
  projection_revision: sequence + 1, kind: 'node_status_changed',
});

describe('runStore', () => {
  it('applies in-order events and asks for a re-read', () => {
    const r = applyEvent(initStream(0), ev(1));
    expect(r.state.lastSeq).toBe(1);
    expect(r.effect).toEqual({ type: 'apply', events: [ev(1)] });
  });
  it('drops duplicates', () => {
    const s = applyEvent(initStream(0), ev(1)).state;
    expect(applyEvent(s, ev(1)).effect).toEqual({ type: 'duplicate' });
  });
  it('buffers a gap, requests catch-up, then drains in order', () => {
    const g = applyEvent(initStream(0), ev(3));
    expect(g.effect).toEqual({ type: 'catchup', after_sequence: 0 });
    const a = applyEvent(g.state, ev(2));
    expect(a.effect).toEqual({ type: 'catchup', after_sequence: 0 });
    const b = applyEvent(a.state, ev(1));
    expect(b.effect).toEqual({ type: 'apply', events: [ev(1), ev(2), ev(3)] });
    expect(b.state.lastSeq).toBe(3);
  });
  it('revision guard drops older projections', () => {
    expect(acceptRevision(5, 4)).toBe(false);
    expect(acceptRevision(5, 5)).toBe(true);
    expect(acceptRevision(5, 6)).toBe(true);
  });
});
