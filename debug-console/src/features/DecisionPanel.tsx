import { useEffect, useRef, useState } from 'react';
import { api, ApiError } from '../api/client';

type Phase = 'idle' | 'needs_step_up' | 'requested' | 'running' | 'succeeded' | 'failed' | 'unknown';

/** Human release decision. 202 means "requested"; only the command receipt confirms. */
export function DecisionPanel() {
  const [available, setAvailable] = useState<string[] | null>(null);
  const [note, setNote] = useState('');
  const [phase, setPhase] = useState<Phase>('idle');
  const key = useRef(crypto.randomUUID()); // idempotency key generated when the form opens
  const statusUrl = useRef('');

  useEffect(() => { api.decision().then((d) => setAvailable(d.available_commands)).catch(() => setAvailable(null)); }, []);

  async function submit() {
    try {
      const acc = await api.respond(note, key.current);
      statusUrl.current = acc.status_url;
      setPhase('requested');
      for (let i = 0; i < 20; i += 1) {
        const c = await api.command(statusUrl.current);
        setPhase(c.state === 'succeeded' ? 'succeeded' : c.state === 'failed' ? 'failed' : 'running');
        if (c.state === 'succeeded' || c.state === 'failed') return;
        await new Promise((r) => setTimeout(r, 200));
      }
      setPhase('unknown');
    } catch (e) {
      if (e instanceof ApiError && e.code === 'waiting_human_reauthentication') setPhase('needs_step_up');
      else setPhase('failed');
    }
  }
  async function stepUp() { await api.stepUp(); await submit(); }

  if (available === null) return <section aria-label="Decisión"><h2>Decisión</h2><p>unknown · decisión no disponible</p></section>;
  const canApprove = available.includes('approve');
  return (
    <section aria-label="Decisión">
      <h2>Decisión</h2>
      <label>Nota <textarea value={note} onChange={(e) => setNote(e.target.value)} /></label>
      {canApprove
        ? <button type="button" onClick={() => void submit()}>Aprobar</button>
        : <button type="button" disabled aria-describedby="why">Aprobar</button>}
      {!canApprove && <span id="why"> Sin permiso: approve no está en available_commands</span>}
      {phase === 'needs_step_up' && (
        <div role="alert">Se requiere reautenticación humana. <button type="button" onClick={() => void stepUp()}>Reautenticar</button></div>
      )}
      <p data-testid="decision-phase" data-phase={phase} role="status">
        {phase === 'requested' && 'Solicitada (aún no confirmada)'}
        {phase === 'running' && 'En ejecución (aún no confirmada)'}
        {phase === 'succeeded' && 'Confirmada por el recibo del comando'}
        {phase === 'failed' && 'Falló'}
        {phase === 'unknown' && 'Desconocido: sin confirmación'}
      </p>
    </section>
  );
}
