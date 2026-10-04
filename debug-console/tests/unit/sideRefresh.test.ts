import { describe, expect, it, vi } from 'vitest';
import { createDebounced, needsSideRefresh } from '../../src/state/sideRefresh';

const ev = (kind: string, sequence = 1) => ({ event_id: `e${sequence}`, run_id: 'r', sequence, entity_ref: { kind: 'run', id: 'r' }, projection_revision: sequence, kind });

describe('needsSideRefresh', () => {
  it('flags doubles_declared as a profile refresh and gates_set as a gates refresh', () => {
    expect(needsSideRefresh([ev('doubles_declared')])).toEqual({ profile: true, gates: false });
    expect(needsSideRefresh([ev('gates_set')])).toEqual({ profile: false, gates: true });
    expect(needsSideRefresh([ev('doubles_declared'), ev('gates_set', 2)])).toEqual({ profile: true, gates: true });
  });
  it('flags the panel events a run streams (investigation, alternatives, diff, decision) as an outcome refresh, like gates_set', () => {
    for (const k of ['investigation_set', 'alternatives_set', 'diff_set', 'decision_set']) expect(needsSideRefresh([ev(k)]), k).toEqual({ profile: false, gates: true });
  });
  it('ignores every other event kind', () => {
    expect(needsSideRefresh([ev('node_status_changed'), ev('run_started', 2)])).toEqual({ profile: false, gates: false });
  });
});

describe('createDebounced', () => {
  it('coalesces a burst into one trailing call and can be cancelled', () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const d = createDebounced(fn, 100);
    d(); d(); d();
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(1);
    d.cancel(); d(); d.cancel();
    vi.advanceTimersByTime(500);
    expect(fn).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });
});
