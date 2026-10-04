import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { AutomationView } from '../../src/features/automation/AutomationView';
import type { AutomationApi } from '../../src/features/automation/api';
import list from '../../fixtures/automation/case-types.json';
import cobro from '../../fixtures/automation/detail-cobro_indebido.json';
import cargo from '../../fixtures/automation/detail-cargo_no_reconocido.json';
import app from '../../fixtures/automation/detail-problema_app.json';

afterEach(cleanup);

const details: Record<string, unknown> = { cobro_indebido: cobro, cargo_no_reconocido: cargo, problema_app: app };
function fake(over: Partial<AutomationApi> = {}): AutomationApi & { approve: ReturnType<typeof vi.fn>; publishStaging: ReturnType<typeof vi.fn> } {
  const approve = vi.fn(async () => ({ state: 'approved_simulated', simulated: true, proposal_id: 'prop-cobro-001' }));
  const publishStaging = vi.fn(async () => ({ state: 'staged_simulated', simulated: true, proposal_id: 'prop-cobro-001' }));
  return { caseTypes: async () => list, caseType: async (id: string) => details[id] ?? cobro, approve, publishStaging, ...over } as never;
}

describe('AutomationView (Automatizacion replica)', () => {
  it('shows the title, the SIMULATED header and the banner that proposes an agent for Cobro indebido', async () => {
    render(<AutomationView api={fake()} />);
    expect(await screen.findByRole('heading', { level: 1, name: 'Automatización' })).toBeTruthy();
    expect(screen.getByTestId('auto-simulated-notice').textContent).toMatch(/SIMULAD/);
    const b = await screen.findByTestId('auto-banner');
    expect(b.textContent).toContain('El sistema propone un agente para Cobro indebido');
    expect(b.textContent).toContain('8 de cada 10 borradores');
    expect(within(b).getByRole('button', { name: 'Ver propuesta' })).toBeTruthy();
  });

  it('lists the six case types with stage, what is measured now, cases today and a source badge on every number', async () => {
    render(<AutomationView api={fake()} />);
    const rows = await screen.findAllByTestId(/^auto-row-/);
    expect(rows).toHaveLength(6);
    const cobroRow = screen.getByTestId('auto-row-cobro_indebido');
    expect(cobroRow.textContent).toContain('Etapa 3');
    expect(cobroRow.textContent).toContain('84 de cada 100 borradores');
    expect(cobroRow.textContent).toContain('21');
    expect(within(cobroRow).getByText('Agente propuesto')).toBeTruthy();
    expect(within(screen.getByTestId('auto-measure-cobro_indebido')).getByTestId('source-badge').textContent).toContain('Flujo simulado');
    const cargoRow = screen.getByTestId('auto-row-cargo_no_reconocido');
    expect(cargoRow.textContent).toContain('Con agente');
    expect(cargoRow.textContent).toContain('resolvió 17 de 26 y pasó 7 a personas');
    expect(screen.getByTestId('auto-row-problema_app').textContent).toContain('Etapa 2');
    expect(screen.getByTestId('auto-row-tarjeta_virtual').textContent).toContain('Etapa 0');
    expect(within(screen.getByTestId('auto-row-problema_app')).getByRole('button', { name: /Ver señales/ })).toBeTruthy();
  });

  it('opens the drawer with how it matured, the last-100 drafts and the thresholds with today values; Escape closes it', async () => {
    render(<AutomationView api={fake()} />);
    fireEvent.click(await within(await screen.findByTestId('auto-row-cobro_indebido')).findByText('Agente propuesto'));
    const d = await screen.findByRole('dialog', { name: /Cobro indebido/ });
    expect(d.textContent).toContain('Cómo maduró');
    expect(d.textContent).toContain('El copiloto responde');
    expect(d.textContent).toContain('4 de agosto');
    expect(d.textContent).toContain('Propuesto hoy');
    expect(d.textContent).toContain('61 tal cual');
    expect(d.textContent).toContain('23 con cambios menores');
    expect(d.textContent).toContain('16 descartados');
    expect(d.textContent).toContain('Umbrales para pasar de etapa');
    expect(d.textContent).toContain('Hoy: 41 casos con preguntas parecidas');
    expect(d.textContent).toContain('Hoy: 84 de 100');
    expect(within(d).getByRole('button', { name: 'Ver propuesta del agente' })).toBeTruthy();
    fireEvent.keyDown(d, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /Cobro indebido/ })).toBeNull());
  });

  it('says what cannot be computed instead of inventing it', async () => {
    render(<AutomationView api={fake()} />);
    fireEvent.click(await within(await screen.findByTestId('auto-row-cargo_no_reconocido')).findByRole('button', { name: /Ver señales/ }));
    const d = await screen.findByRole('dialog', { name: /Cargo no reconocido/ });
    expect(d.textContent).toContain('No calculable');
  });

  it('proposal view names the real target artifact, links the investigation and approves/publishes as SIMULATED', async () => {
    const api = fake();
    render(<AutomationView api={api} />);
    fireEvent.click(within(await screen.findByTestId('auto-banner')).getByRole('button', { name: 'Ver propuesta' }));
    const p = await screen.findByTestId('auto-proposal');
    expect(p.textContent).toContain('copiloto-asesor@staging');
    expect(p.textContent).toContain('p/copiloto@1.0.0');
    expect(p.textContent).toContain('p/copiloto@1.1.0');
    expect(p.textContent).toContain('recepcion');
    expect(p.textContent).toContain('mechanism_proxy');
    expect(within(p).getByRole('link', { name: /ejecución/i }).getAttribute('href')).toBe('#/run/run-cobro-001');
    const publish = within(p).getByRole('button', { name: /Publicar en staging/ }) as HTMLButtonElement;
    expect(publish.disabled).toBe(true);
    const approve = within(p).getByRole('button', { name: /Aprobar \(SIMULADO\)/ });
    fireEvent.click(approve);
    await waitFor(() => expect(p.textContent).toContain('Aprobada (simulada)'));
    expect(api.approve).toHaveBeenCalledWith('cobro_indebido', 'sha256:0f3a9c51d1b7e2a4c6b8d0e2f4a6c8e0a2b4d6f8091a2b3c4d5e6f708192a3b4');
    fireEvent.click(within(p).getByRole('button', { name: /Publicar en staging/ }));
    await waitFor(() => expect(p.textContent).toContain('En staging (simulado)'));
    expect(api.publishStaging).toHaveBeenCalledWith('cobro_indebido');
  });

  it('shows an honest error when the read model is unavailable', async () => {
    render(<AutomationView api={fake({ caseTypes: async () => { throw new Error('x'); } })} />);
    expect(await screen.findByRole('alert')).toBeTruthy();
  });
});
