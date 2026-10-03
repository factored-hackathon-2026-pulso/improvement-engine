import { useEffect, useRef, useState } from 'react';
import { api, ApiError } from '../api/client';
import { t } from '../i18n/es419';

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

  if (available === null) return <section aria-label={t('dec.title')}><h2>{t('dec.title')}</h2><p>{t('dec.unavailable')}</p></section>;
  const canApprove = available.includes('approve');
  return (
    <section aria-label={t('dec.title')}>
      <h2>{t('dec.title')}</h2>
      {canApprove ? (
        <>
          <label>{t('dec.note')} <textarea value={note} onChange={(e) => setNote(e.target.value)} /></label>
          <button type="button" disabled={phase === 'requested' || phase === 'running'} onClick={() => void submit()}>{t('dec.approve')}</button>
        </>
      ) : (
        // No role-name inference: the control is absent whenever the server does not offer the command.
        <p>{t('dec.noPermission')}</p>
      )}
      {phase === 'needs_step_up' && (
        <div role="alert">{t('dec.stepUpNeeded')} <button type="button" onClick={() => void stepUp()}>{t('dec.stepUp')}</button></div>
      )}
      <p data-testid="decision-phase" data-phase={phase} role="status">
        {phase === 'requested' && t('dec.requested')}
        {phase === 'running' && t('dec.running')}
        {phase === 'succeeded' && t('dec.succeeded')}
        {phase === 'failed' && t('dec.failed')}
        {phase === 'unknown' && t('dec.unknown')}
      </p>
    </section>
  );
}
