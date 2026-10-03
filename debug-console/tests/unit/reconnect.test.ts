import { describe, expect, it } from 'vitest';
import { backoffDelay, classifyClose, parseSseFrames } from '../../src/api/reconnect';

describe('backoffDelay (plan 16.13.4: 1-30 s with jitter)', () => {
  it('never goes below 1 s, even with zero jitter', () => {
    for (let a = 0; a < 40; a += 1) expect(backoffDelay(a, () => 0)).toBeGreaterThanOrEqual(1000);
  });
  it('grows exponentially from 1 s and caps at 30 s', () => {
    const max = () => 1;
    expect(backoffDelay(0, max)).toBe(1000);
    expect(backoffDelay(1, max)).toBe(2000);
    expect(backoffDelay(3, max)).toBe(8000);
    expect(backoffDelay(5, max)).toBe(30000);
    expect(backoffDelay(50, max)).toBe(30000);
  });
  it('applies jitter inside [1 s, ceiling]', () => {
    expect(backoffDelay(3, () => 0.5)).toBe(4500);
    expect(backoffDelay(20, () => 0.5)).toBe(15500);
    for (let a = 0; a < 40; a += 1) expect(backoffDelay(a, Math.random)).toBeLessThanOrEqual(30000);
  });
});

describe('classifyClose', () => {
  it('maps statuses to actions', () => {
    expect(classifyClose(410)).toBe('resnapshot');
    expect(classifyClose(401)).toBe('session_expired');
    expect(classifyClose(403)).toBe('forbidden');
    expect(classifyClose(404)).toBe('forbidden');
    expect(classifyClose(0)).toBe('retry');
    expect(classifyClose(503)).toBe('retry');
  });
});

describe('parseSseFrames', () => {
  it('splits frames, keeps the remainder, ignores comments, tracks ids', () => {
    const r = parseSseFrames(': open\n\nid: 3\ndata: {"a":1}\n\nid: 4\ndata: {"a"');
    expect(r.frames).toEqual([{ id: '3', data: '{"a":1}' }]);
    expect(r.rest).toBe('id: 4\ndata: {"a"');
  });
  it('joins multi-line data and handles CRLF', () => {
    const r = parseSseFrames('data: {"a":\r\ndata: 1}\r\n\r\n');
    expect(r.frames).toEqual([{ id: null, data: '{"a":1}' }]);
  });
});
