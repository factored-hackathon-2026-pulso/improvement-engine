import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError, onSessionExpired, setCsrf } from '../../src/api/client';

afterEach(() => { vi.unstubAllGlobals(); setCsrf(''); });
const problem = (code: string, status: number, extra: object = {}) => new Response(
  JSON.stringify({ code, message: code, correlation_id: 'c', retryable: false, ...extra }), { status });

describe('session handling in the client', () => {
  it('never sends a mutation with an empty CSRF token', async () => {
    const f = vi.fn();
    vi.stubGlobal('fetch', f);
    setCsrf('');
    await expect(api.respond('n', 'k', 1)).rejects.toMatchObject({ code: 'csrf_missing' });
    await expect(api.stepUp()).rejects.toMatchObject({ code: 'csrf_missing' });
    expect(f).not.toHaveBeenCalled();
  });
  it('a 401 on a plain request notifies session-expired listeners and drops the CSRF token', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => problem('session_expired', 401)));
    const seen = vi.fn();
    const off = onSessionExpired(seen);
    setCsrf('tok');
    await expect(api.runs()).rejects.toBeInstanceOf(ApiError);
    expect(seen).toHaveBeenCalledTimes(1);
    off();
    await expect(api.runs()).rejects.toBeInstanceOf(ApiError);
    expect(seen).toHaveBeenCalledTimes(1);
    const f = vi.fn();
    vi.stubGlobal('fetch', f);
    await expect(api.stepUp()).rejects.toMatchObject({ code: 'csrf_missing' }); // token was dropped
  });
  it('a transport failure is a typed error, not an unhandled rejection', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => { throw new TypeError('Failed to fetch'); }));
    await expect(api.runs()).rejects.toMatchObject({ status: 0, code: 'network_error' });
  });
  it('exposes the 409 conflict (expected vs current revision) without retrying', async () => {
    const f = vi.fn(async () => problem('stale_revision', 409, { conflict: { expected_revision: 1, current_revision: 3, diff_ref: null } }));
    vi.stubGlobal('fetch', f);
    setCsrf('tok');
    const e = await api.respond('n', 'k', 1).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(ApiError);
    expect((e as ApiError).conflict).toEqual({ expected_revision: 1, current_revision: 3, diff_ref: null });
    expect(f).toHaveBeenCalledTimes(1);
  });
  it('sends expected_revision (decision domain revision) in the response body', async () => {
    const f = vi.fn(async () => new Response(JSON.stringify({ command_ref: { kind: 'external_command', id: 'c' }, status_url: '/s', entity_ref: { kind: 'decision', id: 'd' } }), { status: 202 }));
    vi.stubGlobal('fetch', f);
    setCsrf('tok');
    await api.respond('n', 'k', 7);
    const init = (f.mock.calls[0] as unknown as [string, RequestInit])[1];
    expect(JSON.parse(init.body as string)).toEqual({ expected_revision: 7, response: 'approve', note: 'n' });
  });
});
