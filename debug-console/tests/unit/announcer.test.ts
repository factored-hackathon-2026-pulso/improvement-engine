import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createAnnouncer, diffNodes } from '../../src/a11y/announcer';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('createAnnouncer (one rate-limited live region)', () => {
  it('speaks the first message immediately', () => {
    const out: string[] = [];
    createAnnouncer((t) => out.push(t), 2000).announce('uno');
    expect(out).toEqual(['uno']);
  });
  it('coalesces a burst into one later update and drops duplicates', () => {
    const out: string[] = [];
    const a = createAnnouncer((t) => out.push(t), 2000);
    a.announce('uno');
    for (let i = 0; i < 50; i += 1) { a.announce('dos'); a.announce('tres'); }
    expect(out).toEqual(['uno']);
    vi.advanceTimersByTime(2000);
    expect(out).toEqual(['uno', 'dos. tres']);
    vi.advanceTimersByTime(10000);
    expect(out).toHaveLength(2); // nothing pending: no extra updates
  });
  it('never updates more often than the interval', () => {
    const out: string[] = [];
    const a = createAnnouncer((t) => out.push(t), 2000);
    a.announce('a');
    vi.advanceTimersByTime(1000);
    a.announce('b');
    expect(out).toEqual(['a']);
    vi.advanceTimersByTime(1000);
    expect(out).toEqual(['a', 'b']);
  });
  it('re-announces an identical message only after the region moved on', () => {
    const out: string[] = [];
    const a = createAnnouncer((t) => out.push(t), 1000);
    a.announce('x'); vi.advanceTimersByTime(1000);
    a.announce('x');
    expect(out).toEqual(['x', 'x']);
  });
});

describe('diffNodes', () => {
  const n = (id: string, status: string) => ({ node_id: id, label: id.toUpperCase(), status });
  it('reports only nodes whose status changed, never the whole graph', () => {
    const prev = [n('a', 'running'), n('b', 'planned'), n('c', 'planned')];
    const next = [n('a', 'running'), n('b', 'running'), n('c', 'planned')];
    expect(diffNodes(prev, next)).toEqual([{ label: 'B', status: 'running' }]);
  });
  it('is empty on the first graph (no previous) and when nothing changed', () => {
    expect(diffNodes(null, [n('a', 'running')])).toEqual([]);
    expect(diffNodes([n('a', 'running')], [n('a', 'running')])).toEqual([]);
  });
  it('reports a newly appearing node', () => {
    expect(diffNodes([n('a', 'running')], [n('a', 'running'), n('z', 'queued')])).toEqual([{ label: 'Z', status: 'queued' }]);
  });
});
