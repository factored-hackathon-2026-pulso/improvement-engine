import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/react';
import * as S from '../../src/api/schemas';
import { AlternativesPanel, AttemptHistory, HypothesesList, NativeReport } from '../../src/features/DemoPanels';
import { ModeBanner } from '../../src/app/ModeBanner';
import { hypothesesOf } from '../../src/features/demoModel';

const world = JSON.parse(readFileSync('fixtures/demo-world.json', 'utf8'));
const env = { schema_version: '1', tenant_id: 't', projection_revision: 1, status: 'ok', blocking_reasons: [], available_commands: [] };
afterEach(cleanup);

describe('AlternativesPanel', () => {
  const items = world.demo.alternatives.map((a: unknown) => S.Alternatives.parse({ ...env, items: [a] }).items[0]);
  it('shows do nothing next to the proposed change with expected abandonment and risk, as an accessible table', () => {
    render(<AlternativesPanel items={items} />);
    const panel = screen.getByTestId('alternatives');
    const rows = within(panel).getAllByRole('row');
    expect(rows).toHaveLength(3); // header + 2
    expect(screen.getByTestId('alt-alt-0').getAttribute('data-kind')).toBe('do_nothing');
    expect(screen.getByTestId('alt-alt-0').textContent).toContain('1415');
    expect(screen.getByTestId('alt-alt-1').getAttribute('data-kind')).toBe('proposed_change');
    expect(screen.getByTestId('alt-alt-1').textContent).toContain('guard exposure 0.0025');
    expect(panel.textContent).toContain('Hacer nada');
  });
  it('renders nothing when the run has no alternatives', () => {
    const { container } = render(<AlternativesPanel items={[]} />);
    expect(container.textContent).toBe('');
  });
});

describe('AttemptHistory', () => {
  const attempts = world.demo.attempts.map((a: unknown) => S.GateAttempt.parse(a));
  it('candidate 1 failed -> automatic revision -> candidate 2 passed; the failed attempt is never shown as pass', () => {
    render(<AttemptHistory attempts={attempts} />);
    const a1 = screen.getByTestId('attempt-1');
    const a2 = screen.getByTestId('attempt-2');
    expect(a1.getAttribute('data-outcome')).toBe('fail');
    expect(a1.textContent).toContain('guard_breach');
    expect(a1.textContent).not.toMatch(/Superado|Resultado: pass/);
    expect(a2.getAttribute('data-outcome')).toBe('pass');
    expect(a2.textContent).toContain('revisión automática de cand-1');
    expect(a1.compareDocumentPosition(a2) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
  it('an attempt with a passing native gate but a failing improvement gate is fail', () => {
    render(<AttemptHistory attempts={[attempts[0]]} />);
    expect(screen.getByTestId('attempt-1').getAttribute('data-outcome')).toBe('fail');
  });
});

describe('HypothesesList', () => {
  it('lists every hypothesis with its own verdict; the refuted decoy keeps its counterevidence visible', () => {
    const inv = S.Investigation.parse({ ...env, ...world.investigation['run-demo'] });
    render(<HypothesesList hypotheses={hypothesesOf(inv)} />);
    expect(screen.getByTestId('hyp-main').getAttribute('data-verdict')).toBe('supported');
    const decoy = screen.getByTestId('hyp-card_replacement/otp_verify');
    expect(decoy.getAttribute('data-verdict')).toBe('refuted');
    expect(decoy.textContent).toContain('4 of 7 weeks only');
    expect(screen.getAllByTestId(/^hyp-/)).toHaveLength(3);
  });
});

describe('NativeReport', () => {
  it('shows the report_ref of the native gate (id and digest)', () => {
    const ref = world.gates.native.report_ref;
    render(<NativeReport reportRef={ref} />);
    expect(screen.getByTestId('native-report').textContent).toContain(ref.id);
    expect(screen.getByTestId('native-report').textContent).toContain(ref.digest.slice(0, 12));
  });
  it('says plainly when there is no report', () => {
    render(<NativeReport reportRef={null} />);
    expect(screen.getByTestId('native-report').textContent).toContain('sin informe');
  });
});

describe('ModeBanner stand-in', () => {
  const profile = S.Profile.parse({ target: 'demo-standin', pin: null, ...Object.fromEntries(['runtime_profile', 'doubles', 'doubles_detail'].map((k) => [k, world.demo[k]])), mode: 'stand_in', decision_hook: 'pending' });
  it('is a visible demo banner that labels the stand-in engine, scripted llm and fixture API honestly', () => {
    render(<ModeBanner profile={profile} simulated={false} provider="fixture" />);
    const b = screen.getByTestId('mode-banner');
    expect(b.getAttribute('data-level')).toBe('stand_in');
    expect(b.textContent).toContain('DEMO');
    expect(b.textContent).toContain('motor simulado');
    expect(b.textContent).toContain('LLM con guion');
    expect(b.textContent).toContain('API de fixture');
    expect(b.textContent).toContain('real_local_core_standin');
    expect(b.textContent).not.toMatch(/verificado/i);
  });
  it('lists each double with what it replaces and until when', () => {
    render(<ModeBanner profile={profile} simulated={false} provider="fixture" />);
    const d = screen.getByTestId('doubles-detail');
    expect(d.textContent).toContain('Codex engine stand-in');
    expect(d.textContent).toContain('Rust engine HTTP API exists');
  });
  it('the hook state is stated: human decision not connected', () => {
    render(<ModeBanner profile={profile} simulated={false} provider="fixture" />);
    expect(screen.getByTestId('mode-banner').textContent).toContain('decisión humana: gancho pendiente');
  });
});
