import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../../src/api/client';

const env = {
  schema_version: '1', tenant_id: 't', projection_revision: 1, status: 'ok', blocking_reasons: [], available_commands: [],
};
afterEach(() => vi.unstubAllGlobals());

describe('api client scrubs every response', () => {
  it('masks secrets, reports security.unredacted_payload, never echoes the value', async () => {
    const jws = 'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJlXzEyMzQ1';
    const body = { ...env, hypothesis: `leak ${jws}`, verifier: 'refuted', evidence: [] };
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(body), { status: 200 })));
    const err = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const r = await api.investigation('run-1');
    expect(JSON.stringify(r)).not.toContain('eyJ');
    expect(err).toHaveBeenCalledTimes(1);
    expect(err.mock.calls[0]?.[0]).toBe('security.unredacted_payload');
    expect(JSON.stringify(err.mock.calls)).not.toContain('eyJ');
  });
  it('sends no Authorization header and no JWS-shaped URL', async () => {
    const f = vi.fn(async () => new Response(JSON.stringify({ ...env, items: [], next_cursor: null }), { status: 200 }));
    vi.stubGlobal('fetch', f);
    await api.runs();
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).not.toMatch(/eyJ/);
    expect(JSON.stringify(init.headers ?? {})).not.toMatch(/authorization/i);
    expect(init.credentials).toBe('same-origin');
  });
});
