import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { DecisionPanel } from '../../src/features/DecisionPanel';
import { setCsrf } from '../../src/api/client';

const env = { schema_version: '1', tenant_id: 't', projection_revision: 1, status: 'pending', blocking_reasons: [] };
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
const problem = (code: string, status: number, extra: object = {}) => json(status, { code, message: code, correlation_id: 'c', retryable: false, ...extra });
const accepted = () => json(202, { command_ref: { kind: 'external_command', id: 'c1' }, status_url: '/internal/v1/debug/commands/external_command/c1', entity_ref: { kind: 'decision', id: 'dec-1' } });

type Route = (url: string, init?: RequestInit) => Response | undefined;
function stub(route: Route) {
  const calls: { url: string; init?: RequestInit }[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    return route(url, init) ?? problem('not_found', 404);
  }));
  return calls;
}
const decision = (rev: number, stepUp = false) =>
  json(200, { ...env, available_commands: ['approve'], needs_step_up: stepUp, domain_revision: rev });
const phase = () => screen.getByTestId('decision-phase').getAttribute('data-phase');
afterEach(() => { cleanup(); vi.unstubAllGlobals(); setCsrf(''); });

describe('DecisionPanel', () => {
  it('F21: a stale expected_revision shows the conflict and does not silently resubmit', async () => {
    setCsrf('tok');
    const calls = stub((url, init) => {
      if (url.endsWith('/decisions/dec-1')) return decision(1);
      if (init?.method === 'POST') return problem('stale_revision', 409, { conflict: { expected_revision: 1, current_revision: 3, diff_ref: null } });
      return undefined;
    });
    render(<DecisionPanel />);
    fireEvent.click(await screen.findByRole('button', { name: 'Aprobar' }));
    const stale = await screen.findByTestId('decision-stale');
    expect(stale.textContent).toContain('1');
    expect(stale.textContent).toContain('3');
    expect(phase()).toBe('stale');
    expect(calls.filter((c) => c.init?.method === 'POST')).toHaveLength(1);
  });
  it('F21: reloading re-reads the decision; the next approve carries the new revision and a new key', async () => {
    setCsrf('tok');
    let rev = 1;
    const keys: string[] = [];
    const bodies: { expected_revision: number }[] = [];
    stub((url, init) => {
      if (url.endsWith('/decisions/dec-1')) return decision(rev);
      if (init?.method === 'POST') {
        keys.push((init.headers as Record<string, string>)['idempotency-key']!);
        bodies.push(JSON.parse(init.body as string) as { expected_revision: number });
        if (bodies.length === 1) { rev = 3; return problem('stale_revision', 409, { conflict: { expected_revision: 1, current_revision: 3, diff_ref: null } }); }
        return accepted();
      }
      if (url.includes('/commands/')) return json(200, { state: 'succeeded' });
      return undefined;
    });
    render(<DecisionPanel />);
    fireEvent.click(await screen.findByRole('button', { name: 'Aprobar' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Recargar decisión' }));
    await waitFor(() => expect(screen.queryByTestId('decision-stale')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'Aprobar' }));
    await waitFor(() => expect(phase()).toBe('succeeded'));
    expect(bodies.map((b) => b.expected_revision)).toEqual([1, 3]);
    expect(new Set(keys).size).toBe(2);
  });
  it('a failing step-up is reported and does not submit the approval again', async () => {
    setCsrf('tok');
    const calls = stub((url, init) => {
      if (url.endsWith('/decisions/dec-1')) return decision(1, true);
      if (url.endsWith('/responses')) return problem('waiting_human_reauthentication', 403);
      if (url.endsWith('/auth/step-up') && init?.method === 'POST') return problem('step_up_unavailable', 503);
      return undefined;
    });
    render(<DecisionPanel />);
    fireEvent.click(await screen.findByRole('button', { name: 'Aprobar' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Reautenticar' }));
    await waitFor(() => expect(phase()).toBe('step_up_failed'));
    expect(calls.filter((c) => c.url.endsWith('/responses'))).toHaveLength(1);
  });
  it('a step-up that cannot be sent (missing CSRF) is reported, not thrown', async () => {
    setCsrf('tok');
    stub((url) => {
      if (url.endsWith('/decisions/dec-1')) return decision(1, true);
      if (url.endsWith('/responses')) return problem('waiting_human_reauthentication', 403);
      return undefined;
    });
    render(<DecisionPanel />);
    fireEvent.click(await screen.findByRole('button', { name: 'Aprobar' }));
    const btn = await screen.findByRole('button', { name: 'Reautenticar' });
    setCsrf('');
    fireEvent.click(btn);
    await waitFor(() => expect(phase()).toBe('step_up_failed'));
  });
});
