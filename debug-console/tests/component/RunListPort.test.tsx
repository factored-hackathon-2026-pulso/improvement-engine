import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { RunList } from '../../src/features/RunList';
import { ProviderDeclaration } from '../../src/app/ProviderDeclaration';
import { DebugApiProvider } from '../../src/api/debug/context';
import { DebugApiError } from '../../src/api/debug/dto';
import type { DebugApi } from '../../src/api/debug/port';

const env = { schema_version: '1' as const, tenant_id: 't', projection_revision: 1, status: 'ok', blocking_reasons: [], available_commands: [], next_cursor: null };
const stub = (over: Partial<DebugApi>): DebugApi => over as DebugApi;
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe('RunList depends only on the DebugApi port', () => {
  it('renders runs from a stub port and never calls fetch itself', async () => {
    const f = vi.fn(); vi.stubGlobal('fetch', f);
    const api = stub({ runs: async () => ({ ...env, items: [{ run_id: 'r1', title: 'Run one', state: 'running', origin: 'scheduled', projection_revision: 2 }] }) });
    render(<DebugApiProvider api={api}><RunList /></DebugApiProvider>);
    expect((await screen.findByRole('link', { name: 'Run one' })).getAttribute('href')).toBe('#/run/r1');
    expect(f).not.toHaveBeenCalled();
  });
  it('shows the empty state as unknown, not as "no activity"', async () => {
    render(<DebugApiProvider api={stub({ runs: async () => ({ ...env, items: [] }) })}><RunList /></DebugApiProvider>);
    expect(await screen.findByText('Sin ejecuciones: unknown · no implica ingesta completa')).toBeTruthy();
  });
  it('shows the unknown-list error (never an invented last state) when the port fails', async () => {
    const api = stub({ runs: async () => { throw new DebugApiError(503, 'dependency_unavailable', 'x', 'c', true); } });
    render(<DebugApiProvider api={api}><RunList /></DebugApiProvider>);
    expect(await screen.findByText('unknown · no se pudo leer la lista')).toBeTruthy();
  });
});

describe('ProviderDeclaration', () => {
  it('declares provider, target, runtime_profile and doubles[] from the port', async () => {
    const api = stub({ mode: async () => ({ provider: 'stand-in', target: 'demo', runtime_profile: 'standin_profile', doubles: ['stand_in_engine', 'scripted_llm'] }) });
    render(<DebugApiProvider api={api}><ProviderDeclaration /></DebugApiProvider>);
    const el = await screen.findByTestId('provider-declaration');
    expect(el.textContent).toContain('stand-in');
    expect(el.textContent).toContain('demo');
    expect(el.textContent).toContain('standin_profile');
    expect(el.textContent).toContain('stand_in_engine');
    expect(el.getAttribute('data-provider')).toBe('stand-in');
  });
  it('a failed declaration is shown as unverified, never silently real', async () => {
    const api = stub({ mode: async () => { throw new DebugApiError(0, 'network_error', 'x', '', true); } });
    render(<DebugApiProvider api={api}><ProviderDeclaration /></DebugApiProvider>);
    expect((await screen.findByTestId('provider-declaration')).getAttribute('data-provider')).toBe('unverified');
  });
});

describe('provider selection is configuration (public/config.json dataProvider)', () => {
  const stubConfig = (cfg: object) => {
    const f = vi.fn(async (url: string) => (url === '/config.json' ? new Response(JSON.stringify(cfg), { status: 200 }) : new Response('{}', { status: 404 })));
    vi.stubGlobal('fetch', f);
    return f;
  };
  it('stand-in: the run list and the declaration come in-process, with no debug request on the network', async () => {
    const f = stubConfig({ provider: 'fixture', dataProvider: 'stand-in', sseHeartbeatMs: 5000 });
    const { App } = await import('../../src/app/App');
    render(<App />);
    expect((await screen.findByTestId('provider-declaration')).getAttribute('data-provider')).toBe('stand-in');
    expect((await screen.findAllByRole('link', { name: /run/i })).length).toBeGreaterThan(0);
    expect(f.mock.calls.map((c) => String(c[0])).filter((u) => u.includes('/internal/') || u.includes('/api/v1/'))).toEqual([]);
    expect(screen.getByTestId('mode-banner').getAttribute('data-level')).toBe('stand_in');
  });
  it('an unknown dataProvider value falls back to http (the real transport), never to a demo', async () => {
    const { loadConfig } = await import('../../src/api/client');
    stubConfig({ dataProvider: 'bogus' });
    expect((await loadConfig()).dataProvider).toBe('http');
    stubConfig({ dataProvider: 'fixture', apiBase: 'https://control.internal.example' });
    const c = await loadConfig();
    expect([c.dataProvider, c.apiBase]).toEqual(['fixture', 'https://control.internal.example']);
    stubConfig({ apiBase: 'javascript:alert(1)' });
    expect((await loadConfig()).apiBase).toBe('');
  });
  it('a screen still on the legacy client is declared as NOT served by the stand-in/fixture provider', async () => {
    stubConfig({ provider: 'fixture', dataProvider: 'stand-in', sseHeartbeatMs: 5000 });
    window.location.hash = '#/sources';
    const { App } = await import('../../src/app/App');
    render(<App />);
    expect((await screen.findByTestId('provider-partial')).textContent).toContain('cliente heredado');
    window.location.hash = '';
  });
  it('the run list (on the port) carries no partial notice', async () => {
    stubConfig({ provider: 'fixture', dataProvider: 'stand-in', sseHeartbeatMs: 5000 });
    window.location.hash = '';
    const { App } = await import('../../src/app/App');
    render(<App />);
    await screen.findAllByRole('link', { name: /run/i });
    expect(screen.queryByTestId('provider-partial')).toBeNull();
  });
});
