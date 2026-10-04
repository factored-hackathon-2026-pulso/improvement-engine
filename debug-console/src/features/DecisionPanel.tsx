import { useCallback, useEffect, useRef, useState } from 'react';
import type { z } from 'zod';
import { api, ApiError, type Conflict } from '../api/client';
import type * as S from '../api/schemas';
import { useAnnounce } from '../a11y/AnnounceContext';
import { t } from '../i18n/es419';

type Phase = 'idle' | 'needs_step_up' | 'step_up_failed' | 'stale' | 'requested' | 'running' | 'succeeded' | 'failed' | 'unknown';
const PHASE_TEXT: Partial<Record<Phase, Parameters<typeof t>[0]>> = {
  requested: 'dec.requested', running: 'dec.running', succeeded: 'dec.succeeded', failed: 'dec.failed', unknown: 'dec.unknown',
  step_up_failed: 'dec.stepUpFailed',
};

type Card = z.infer<typeof S.RunDecision>['card'];
const gateText = (g: { status: string | null; reason?: string | null } | null) => (g ? `${g.status ?? 'unknown'}${g.reason ? ` (${g.reason})` : ''}` : 'unknown');

/** The decision a run committed (simulated or blocked) with the gate state it was taken on. Read-only: no action is offered. */
function DecisionCard({ card }: { card: Card }) {
  return (
    <div data-testid="decision-card" data-state={card.state}>
      <p><strong>{t('dec.card.simulated')}</strong></p>
      <p>{t('dec.card.state', { state: card.state, proposal: card.proposal_id ?? 'unknown' })}</p>
      <p>{t('dec.card.issuer', { issuer: card.issuer, actor: card.actor ?? 'unknown' })}</p>
      <p>{t('dec.card.gate', { verdict: card.gate.verdict ?? 'unknown', safety: gateText(card.gate.safety), improvement: gateText(card.gate.improvement) })}</p>
      <p>{card.override
        ? t('dec.card.override', { label: card.override.label, by: card.override.by, actor: card.override.actor ?? 'unknown', gate: card.override.of_gate_verdict ?? 'unknown', reason: card.override.reason })
        : t('dec.card.noOverride')}</p>
      <h3>{t('dec.card.reasons')}</h3>
      <ul>{card.reasons.map((r, i) => <li key={i}>{r}</li>)}</ul>
      <p>{t('dec.card.noActions')}</p>
    </div>
  );
}

/** Human release decision. 202 means "requested"; only the command receipt confirms. With a `runId` and a committed decision, the card is shown instead. */
export function DecisionPanel({ hookPending = false, runId, refresh = 0 }: { hookPending?: boolean; runId?: string; refresh?: number }) {
  const announce = useAnnounce();
  const [state, setState] = useState<{ commands: string[]; revision: number } | null>(null);
  const [card, setCard] = useState<Card | null>(null);
  const [note, setNote] = useState('');
  const [phase, setPhase] = useState<Phase>('idle');
  const [conflict, setConflict] = useState<Conflict | null>(null);
  const key = useRef(crypto.randomUUID()); // idempotency key generated when the form opens
  const statusUrl = useRef('');

  const load = useCallback(() => api.decision()
    .then((d) => setState({ commands: d.available_commands, revision: d.domain_revision }))
    .catch(() => setState(null)), []);
  useEffect(() => {
    // The run's own committed decision wins; a run with none (404) keeps the legacy single-decision behaviour.
    if (!runId) { void load(); return; }
    api.runDecision(runId).then((d) => setCard(d.card)).catch(() => { setCard(null); void load(); });
  }, [load, runId, refresh]);
  useEffect(() => {
    const k = PHASE_TEXT[phase];
    if (k) announce(t(k));
  }, [phase, announce]);

  async function submit() {
    if (!state) return;
    try {
      const acc = await api.respond(note, key.current, state.revision);
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
      else if (e instanceof ApiError && e.code === 'stale_revision') { setConflict(e.conflict); setPhase('stale'); announce(t('dec.stale', { expected: e.conflict?.expected_revision ?? state.revision, current: e.conflict?.current_revision ?? '?' })); }
      else setPhase('failed');
    }
  }
  async function stepUp() {
    try { await api.stepUp(); } catch { setPhase('step_up_failed'); return; }
    await submit();
  }
  async function reload() {
    key.current = crypto.randomUUID(); // new target revision -> new intention -> new idempotency key
    setConflict(null); setPhase('idle');
    await load();
  }

  if (card) return <section aria-label={t('dec.card.title')} data-testid="decision"><h2>{t('dec.title')}</h2><DecisionCard card={card} /></section>;
  if (state === null) return <section aria-label={t('dec.title')} data-testid="decision"><h2>{t('dec.title')}</h2><p>{t('dec.unavailable')}</p></section>;
  const canApprove = state.commands.includes('approve');
  return (
    <section aria-label={t('dec.title')} data-testid="decision">
      <h2>{t('dec.title')}</h2>
      {hookPending && <p className="hook" data-testid="decision-hook">{t('dec.hookPending')}</p>}
      {canApprove ? (
        <>
          <label>{t('dec.note')} <textarea value={note} onChange={(e) => setNote(e.target.value)} /></label>
          <button type="button" disabled={phase === 'requested' || phase === 'running' || phase === 'stale'} onClick={() => void submit()}>{t('dec.approve')}</button>
        </>
      ) : (
        // No role-name inference: the control is absent whenever the server does not offer the command.
        <p>{t('dec.noPermission')}</p>
      )}
      {phase === 'needs_step_up' && (
        <div>{t('dec.stepUpNeeded')} <button type="button" onClick={() => void stepUp()}>{t('dec.stepUp')}</button></div>
      )}
      {phase === 'stale' && (
        <div data-testid="decision-stale">
          {t('dec.stale', { expected: conflict?.expected_revision ?? state.revision, current: conflict?.current_revision ?? '?' })}{' '}
          <button type="button" onClick={() => void reload()}>{t('dec.reload')}</button>
        </div>
      )}
      <p data-testid="decision-phase" data-phase={phase}>
        {PHASE_TEXT[phase] ? t(PHASE_TEXT[phase]!) : ''}
      </p>
    </section>
  );
}
