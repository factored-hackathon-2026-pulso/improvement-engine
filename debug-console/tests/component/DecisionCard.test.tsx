import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { DecisionPanel } from '../../src/features/DecisionPanel';
import { setCsrf } from '../../src/api/client';

const env = { schema_version: '1', tenant_id: 't', projection_revision: 1, status: 'ok', blocking_reasons: [], available_commands: [] };
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
const card = {
  state: 'approved', simulated: true, label: 'SIMULATED', issuer: 'simulated-issuer', actor: 'local-supervisor',
  gate: { verdict: 'fail', safety: { status: 'pass', reason: null }, improvement: { status: 'fail', reason: 'no_structural_improvement' } },
  override: { label: 'human_override', by: 'human', actor: 'local-supervisor', reason: 'exercise approve/publish of a failed structural gate; no quality claim', simulated: true, of_gate_verdict: 'fail' },
  reasons: ['Gate verdict was fail; a SIMULATED human override was applied', 'No human took this decision: simulated'], proposal_id: 'prop-1', quality_claims: 'forbidden',
};
const calls: string[] = [];
const stub = (run: (url: string) => Response | undefined) => vi.stubGlobal('fetch', vi.fn(async (url: string) => {
  calls.push(url);
  return run(url) ?? json(404, { code: 'not_found', message: 'x', correlation_id: 'c', retryable: false });
}));
afterEach(() => { cleanup(); vi.unstubAllGlobals(); setCsrf(''); calls.length = 0; });

describe('DecisionPanel with the decision card of the run', () => {
  it('shows the simulated decision with the gate state it was taken on, the override and the reasons, and offers no action', async () => {
    stub((u) => (u.endsWith('/runs/r1/decision') ? json(200, { ...env, decision_id: 'dec-1-x', available_commands: [], needs_step_up: false, domain_revision: 1, card }) : undefined));
    render(<DecisionPanel runId="r1" />);
    const c = await screen.findByTestId('decision-card');
    expect(c.getAttribute('data-state')).toBe('approved');
    expect(c.textContent).toContain('SIMULADA');
    expect(c.textContent).toContain('no_structural_improvement');
    expect(c.textContent).toContain('human_override');
    expect(c.textContent).toContain('No human took this decision');
    expect(screen.queryByRole('button', { name: 'Aprobar' })).toBeNull();
    expect(calls.some((u) => u.includes('/decisions/dec-1'))).toBe(false);
  });
  it('a blocked approval is shown as blocked', async () => {
    stub((u) => (u.endsWith('/runs/r1/decision') ? json(200, { ...env, decision_id: 'dec-b', available_commands: [], needs_step_up: false, domain_revision: 1, card: { ...card, state: 'blocked', actor: null, override: null } }) : undefined));
    render(<DecisionPanel runId="r1" />);
    expect((await screen.findByTestId('decision-card')).getAttribute('data-state')).toBe('blocked');
  });
  it('a run with no committed decision falls back to the legacy decision route', async () => {
    stub((u) => (u.endsWith('/decisions/dec-1') ? json(200, { ...env, status: 'pending', available_commands: ['approve'], needs_step_up: false, domain_revision: 1 }) : undefined));
    render(<DecisionPanel runId="r2" />);
    expect(await screen.findByRole('button', { name: 'Aprobar' })).toBeTruthy();
    expect(screen.queryByTestId('decision-card')).toBeNull();
  });
});
