import { useEffect, useState } from 'react';
import type { z } from 'zod';
import { api } from '../api/client';
import type * as S from '../api/schemas';
import { t } from '../i18n/es419';
import { AlternativesPanel, AttemptHistory, HypothesesList, NativeReport } from './DemoPanels';
import { hypothesesOf } from './demoModel';

type Inv = z.infer<typeof S.Investigation>;
type GatesT = z.infer<typeof S.Gates>;
type DiffT = z.infer<typeof S.Diff>;
type MemT = z.infer<typeof S.Memory>;
type AltsT = z.infer<typeof S.Alternatives>;

const KNOWN_GATE = ['pending', 'pass', 'fail', 'unknown', 'not_evaluable', 'insufficient_power', 'dependency_blocked', 'unsupported', 'failed_infra', 'unsafe'];
export const label = (v: string, known: string[]) => (known.includes(v) ? v : t('enum.unrecognised', { code: v }));

function useLoad<T>(fn: () => Promise<T>, dep: string) {
  const [v, setV] = useState<T | null>(null);
  const [err, setErr] = useState(false);
  useEffect(() => { setV(null); setErr(false); fn().then(setV).catch(() => setErr(true)); }, [dep]); // eslint-disable-line react-hooks/exhaustive-deps
  return { v, err };
}

const RELATIONS: [string, 'inv.contradicts' | 'inv.supports' | 'inv.limits'][] = [['contradicts', 'inv.contradicts'], ['supports', 'inv.supports'], ['limits', 'inv.limits']];

export function Investigation({ runId }: { runId: string }) {
  const { v, err } = useLoad<Inv>(() => api.investigation(runId), runId);
  if (err) return <p>{t('inv.unavailable')}</p>;
  if (!v) return <p>{t('inv.loading')}</p>;
  return (
    <section aria-label={t('inv.title')} data-testid="investigation">
      <h2>{t('inv.title')}</h2>
      <p>{t('inv.hypothesis', { value: v.hypothesis ?? t('inv.noHypothesis') })} <strong data-testid="verifier">{v.verifier}</strong></p>
      <HypothesesList hypotheses={hypothesesOf(v)} />
      {RELATIONS.map(([rel, key]) => (
        <div key={rel} data-testid={`ev-${rel}`}>
          <h3>{t(key)} ({v.evidence.filter((e) => e.relation === rel).length})</h3>
          <ul>
            {v.evidence.filter((e) => e.relation === rel).map((e) => (
              <li key={e.evidence_ref.id} className={rel}>{t(key)}: {e.summary} <small>({e.source_kind}, {e.validation})</small></li>
            ))}
          </ul>
        </div>
      ))}
      {v.evidence.some((e) => !RELATIONS.some(([r]) => r === e.relation)) && <p>{t('inv.unknownRelation')}</p>}
    </section>
  );
}

/** Alternatives, gates (with per-attempt history) and the diff of the proposal that THIS run produced. */
export function RunOutcome({ runId }: { runId: string }) {
  const { v, err } = useLoad<GatesT>(() => api.gates(runId), runId);
  const alts = useLoad<AltsT | null>(() => api.alternatives(runId).catch(() => null), runId);
  return (
    <>
      {alts.v && <AlternativesPanel items={alts.v.items} />}
      {err ? <p>{t('gates.unavailable')}</p> : !v ? <p>{t('gates.loading')}</p> : <Gates v={v} />}
      {err ? <p>{t('diff.unavailable')}</p> : !v ? <p>{t('diff.loading')}</p> : <DiffView proposalId={v.proposal_id ?? null} />}
    </>
  );
}

export function Gates({ v }: { v: GatesT }) {
  return (
    <section aria-label={t('gates.title')} data-testid="gates">
      <h2>{t('gates.title')}</h2>
      <div className="cards">
        <div className="card" data-testid="gate-native" data-status={v.native.status}>
          <h3>{t('gates.native')}</h3><p>{label(v.native.status, KNOWN_GATE)}{v.native.attempt ? ` (${t('gates.attempt', { n: v.native.attempt })})` : ''}</p>
          {v.native.status !== 'not_evaluable' && <NativeReport reportRef={v.native.report_ref} />}
        </div>
        <div className="card" data-testid="gate-improvement" data-status={v.improvement.status}>
          <h3>{t('gates.improvement')}</h3><p>{label(v.improvement.status, KNOWN_GATE)}{v.improvement.reason_code ? ` · ${v.improvement.reason_code}` : ''}</p>
        </div>
        <div className="card" data-testid="gate-combined"><h3>{t('gates.combined')}</h3><p>{v.combined.decision}{v.combined.reason_code ? ` · ${v.combined.reason_code}` : ''}</p></div>
      </div>
      <AttemptHistory attempts={v.attempts ?? []} />
    </section>
  );
}

export function DiffView({ proposalId }: { proposalId: string | null }) {
  if (!proposalId) {
    return <section aria-label={t('diff.section')} data-testid="diff"><h2>{t('diff.title')}</h2><p>{t('diff.noProposal')}</p></section>;
  }
  return <DiffFor id={proposalId} />;
}

function DiffFor({ id }: { id: string }) {
  const { v, err } = useLoad<DiffT>(() => api.diff(id), id);
  if (err) return <p>{t('diff.unavailable')}</p>;
  if (!v) return <p>{t('diff.loading')}</p>;
  return (
    <section aria-label={t('diff.section')} data-testid="diff">
      <h2>{t('diff.title')}</h2>
      <div role="group" aria-label={t('diff.lines')} tabIndex={0}>
        {v.lines.map((l, i) => (
          <pre key={i} className={l.op}>{l.op === 'add' ? '+ ' : l.op === 'del' ? '- ' : '  '}{l.text}</pre>
        ))}
      </div>
    </section>
  );
}

export function MemoryView() {
  const { v, err } = useLoad<MemT>(() => api.memory(), 'mem');
  if (err) return <p>{t('mem.unavailable')}</p>;
  if (!v) return <p>{t('mem.loading')}</p>;
  return (
    <section aria-label={t('mem.title')}>
      <h2>{t('mem.title')}</h2>
      <ul>{v.items.map((m) => (
        <li key={m.memory_id} data-testid={`mem-${m.memory_id}`}>{m.revoked ? t('mem.revoked') : ''}{m.title} <small>[{m.status}]</small></li>
      ))}</ul>
    </section>
  );
}

/** F11: when trace ids are missing (collector down) the panel degrades visibly; the durable timeline stays the truth. */
export function TracePanel({ nodes }: { nodes: { trace_id?: string | null }[] }) {
  const missing = nodes.filter((n) => !n.trace_id).length;
  const degraded = missing > 0;
  return (
    <section aria-label={t('trace.title')} data-testid="trace-panel" data-state={degraded ? 'degraded' : 'ok'} className={degraded ? 'panel-degraded' : undefined}>
      <h2>{t('trace.title')}</h2>
      <p>{degraded ? t('trace.degraded', { n: missing, total: nodes.length }) : t('trace.ok', { n: nodes.length })}</p>
    </section>
  );
}
