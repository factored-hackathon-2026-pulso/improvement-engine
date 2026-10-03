import { describe, expect, it } from 'vitest';
import { backoffDelay, classifyClose, parseSseFrames } from '../../src/api/reconnect';

describe('backoffDelay', () => {
  it('grows exponentially and caps', () => {
    const noJitter = () => 1;
    expect(backoffDelay(0, noJitter)).toBe(500);
    expect(backoffDelay(1, noJitter)).toBe(1000);
    expect(backoffDelay(3, noJitter)).toBe(4000);
    expect(backoffDelay(20, noJitter)).toBe(15000);
  });
  it('applies full jitter within [0, cap]', () => {
    expect(backoffDelay(2, () => 0)).toBe(0);
    expect(backoffDelay(2, () => 0.5)).toBe(1000);
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
