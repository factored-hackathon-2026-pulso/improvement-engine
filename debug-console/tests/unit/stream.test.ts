import { afterEach, describe, expect, it, vi } from 'vitest';
import { streamEvents, type StreamHandlers } from '../../src/api/client';

afterEach(() => vi.unstubAllGlobals());
const enc = new TextEncoder();
const sse = (chunks: string[]) => new Response(new ReadableStream({
  start(c) { chunks.forEach((x) => c.enqueue(enc.encode(x))); c.close(); },
}), { status: 200 });
const handlers = (over: Partial<StreamHandlers> = {}): StreamHandlers => ({
  onEvent: vi.fn(), onOpen: vi.fn(), onDrop: vi.fn(), onResnapshot: vi.fn(), onFatal: vi.fn(), onActivity: vi.fn(), ...over,
});
const done = (ms = 30) => new Promise((r) => setTimeout(r, ms));

describe('streamEvents', () => {
  it('reports activity for comment-only heartbeat frames (no events)', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => sse([': open\n\n', ': hb\n\n'])));
    const h = handlers();
    const stop = streamEvents('r', h, { sleep: () => new Promise(() => undefined) });
    await done();
    stop();
    expect(h.onActivity).toHaveBeenCalled();
    expect(h.onEvent).not.toHaveBeenCalled();
  });
  it('passes the 410 recovery cursor to onResnapshot', async () => {
    const f = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ code: 'cursor_expired', recovery_after_sequence: 77 }), { status: 410 }))
      .mockImplementation(async () => sse([': open\n\n']));
    vi.stubGlobal('fetch', f);
    const h = handlers();
    const stop = streamEvents('r', h, { sleep: () => new Promise(() => undefined) });
    await done(60);
    stop();
    expect(h.onResnapshot).toHaveBeenCalledWith({ recoveryCursor: 77 });
  });
  it('a 410 without a usable body still resnapshots with an unknown cursor', async () => {
    const f = vi.fn()
      .mockResolvedValueOnce(new Response('nope', { status: 410 }))
      .mockImplementation(async () => sse([': open\n\n']));
    vi.stubGlobal('fetch', f);
    const h = handlers();
    const stop = streamEvents('r', h, { sleep: () => new Promise(() => undefined) });
    await done(60);
    stop();
    expect(h.onResnapshot).toHaveBeenCalledWith({ recoveryCursor: null });
  });
});
