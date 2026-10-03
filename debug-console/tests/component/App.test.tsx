import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { App } from '../../src/app/App';
import { setCsrf } from '../../src/api/client';

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
const problem = (code: string, status: number) => json(status, { code, message: code, correlation_id: 'c', retryable: false });
const env = { schema_version: '1', tenant_id: 't', projection_revision: 1, status: 'ok', blocking_reasons: [], available_commands: [] };
const session = { principal: 'p', tenant_id: 't', scopes: [], expires_at: '2099-01-01T00:00:00Z', csrf_token: 'tok', auth: { simulated: false, level: 'basic', auth_at: 'x' } };
const EMPTY = 'Sin ejecuciones: unknown · no implica ingesta completa';

function stub(sessionRes: () => Response, runsRes: () => Response = () => json(200, { ...env, items: [], next_cursor: null })) {
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (url === '/config.json') return json(200, { provider: 'fixture', sseHeartbeatMs: 5000 });
    if (url === '/api/v1/auth/session') return sessionRes();
    if (url.endsWith('/debug/profile')) return json(200, { target: 'mock', runtime_profile: 'fixture', doubles: [], pin: null });
    if (url.endsWith('/debug/runs')) return runsRes();
    return problem('not_found', 404);
  }));
}
afterEach(() => { cleanup(); vi.unstubAllGlobals(); setCsrf(''); });

describe('App session handling', () => {
  it('a failed session fetch is visible, never a silent empty CSRF', async () => {
    stub(() => problem('dependency_unavailable', 503));
    render(<App />);
    expect((await screen.findByTestId('session-banner')).getAttribute('data-state')).toBe('unavailable');
  });
  it('session 401 shows the session-expired state', async () => {
    stub(() => problem('session_expired', 401));
    render(<App />);
    expect((await screen.findByTestId('session-banner')).getAttribute('data-state')).toBe('expired');
  });
  it('a non-SSE 401 mid-session flips the UI to session-expired', async () => {
    stub(() => json(200, session), () => problem('session_expired', 401));
    render(<App />);
    await waitFor(() => expect(screen.getByTestId('session-banner').getAttribute('data-state')).toBe('expired'));
  });
  it('a healthy session renders no session banner', async () => {
    stub(() => json(200, session));
    render(<App />);
    await screen.findByText(EMPTY);
    expect(screen.queryByTestId('session-banner')).toBeNull();
  });
  it('exposes a single live region', async () => {
    stub(() => json(200, session));
    const { container } = render(<App />);
    await screen.findByText(EMPTY);
    expect(container.querySelectorAll('[aria-live],[role=status],[role=alert],[role=log]')).toHaveLength(1);
  });
});

describe("App sources route", () => {
  it("#/sources renders the Sources view and the nav links to it", async () => {
    stub(() => json(200, session));
    window.location.hash = "#/sources";
    render(<App />);
    expect(await screen.findByTestId("sources-standin")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Fuentes" }).getAttribute("href")).toBe("#/sources");
    window.location.hash = "";
  });
});
