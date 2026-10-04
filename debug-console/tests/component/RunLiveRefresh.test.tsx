import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor, act } from '@testing-library/react';
import type { StreamHandlers } from '../../src/api/client';

let handlers: StreamHandlers | null = null;
vi.mock('../../src/api/client', async (orig) => {
  const real = await orig<typeof import('../../src/api/client')>();
  return { ...real, streamEvents: (_id: string, h: StreamHandlers) => { handlers = h; queueMicrotask(() => h.onOpen(false)); return () => {}; } };
});
import { App } from '../../src/app/App';
import { setCsrf } from '../../src/api/client';

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
const env = { schema_version: '1', tenant_id: 't', projection_revision: 1, status: 'ok', blocking_reasons: [], available_commands: [] };
const session = { principal: 'p', tenant_id: 't', scopes: [], expires_at: '2099-01-01T00:00:00Z', csrf_token: 'tok', auth: { simulated: false, level: 'basic', auth_at: 'x' } };
const ev = (kind: string, sequence: number) => ({ event_id: `e${sequence}`, run_id: 'r1', sequence, entity_ref: { kind: 'run', id: 'r1' }, projection_revision: sequence, kind });

let profileDoubles: string[]; let nativeStatus: string; let calls: { profile: number; gates: number };
beforeEach(() => {
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  handlers = null; profileDoubles = ['early_double']; nativeStatus = 'not_evaluable'; calls = { profile: 0, gates: 0 };
  window.location.hash = '#/run/r1';
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (url === '/config.json') return json(200, { provider: 'fixture', sseHeartbeatMs: 5000 });
    if (url === '/api/v1/auth/session') return json(200, session);
    if (url.endsWith('/debug/profile')) { calls.profile++; return json(200, { target: 'mock', runtime_profile: 'demo', doubles: profileDoubles, pin: null, mode: 'stand_in' }); }
    if (url.endsWith('/runs/r1/graph')) return json(200, { ...env, nodes: [] });
    if (url.includes('/runs/r1/events?')) return json(200, { items: [] });
    if (url.endsWith('/runs/r1/gates')) {
      calls.gates++;
      return json(200, { ...env, native: { status: nativeStatus, reason_code: null }, improvement: { status: 'unknown', reason_code: null }, combined: { decision: 'hold', reason_code: null }, proposal_id: null });
    }
    return json(404, { code: 'not_found', message: 'x', correlation_id: 'c', retryable: false });
  }));
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); setCsrf(''); window.location.hash = ''; });

const ready = async () => {
  await waitFor(() => expect(screen.getByTestId('gate-native').getAttribute('data-status')).toBe('not_evaluable'));
  await waitFor(() => expect(handlers).not.toBeNull());
  await waitFor(() => expect(screen.getByTestId('mode-banner').textContent).toContain('early_double'));
  await act(async () => { await new Promise((r) => setTimeout(r, 30)); });
};

describe('run view live side-panels', () => {
  it('refetches the profile when doubles_declared arrives, so the banner is never stale', async () => {
    render(<App />);
    await ready();
    profileDoubles = ['early_double', 'late_double'];
    act(() => handlers!.onEvent(ev('doubles_declared', 1)));
    await waitFor(() => expect(screen.getByTestId('mode-banner').textContent).toContain('late_double'));
  });
  it('refetches the gates when gates_set arrives: never shows not_evaluable after it', async () => {
    render(<App />);
    await ready();
    nativeStatus = 'fail';
    act(() => handlers!.onEvent(ev('gates_set', 1)));
    await waitFor(() => expect(screen.getByTestId('gate-native').getAttribute('data-status')).toBe('fail'));
  });
  it('debounces a burst of events into one refetch and ignores unrelated events', async () => {
    render(<App />);
    await ready();
    const before = { ...calls };
    act(() => { for (let i = 1; i <= 5; i++) handlers!.onEvent(ev(i % 2 ? 'doubles_declared' : 'gates_set', i)); handlers!.onEvent(ev('node_status_changed', 6)); });
    await act(async () => { await new Promise((r) => setTimeout(r, 600)); });
    expect(calls.profile - before.profile).toBe(1);
    expect(calls.gates - before.gates).toBe(1);
  });
  it('does not blank the Gates panel or move focus while refetching', async () => {
    render(<App />);
    await ready();
    const btn = document.createElement('button'); document.body.appendChild(btn); btn.focus();
    act(() => handlers!.onEvent(ev('gates_set', 1)));
    expect(screen.queryByText('Cargando gates…')).toBeNull();
    expect(document.activeElement).toBe(btn);
    btn.remove();
  });
});
