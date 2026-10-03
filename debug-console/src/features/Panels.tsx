import { useEffect, useState } from 'react';
import type { z } from 'zod';
import { api } from '../api/client';
import type * as S from '../api/schemas';
import { t } from '../i18n/es419';

type Inv = z.infer<typeof S.Investigation>;
type GatesT = z.infer<typeof S.Gates>;
type DiffT = z.infer<typeof S.Diff>;
type MemT = z.infer<typeof S.Memory>;

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

export function Gates({ runId }: { runId: string }) {
  const { v, err } = useLoad<GatesT>(() => api.gates(runId), runId);
  if (err) return <p>{t('gates.unavailable')}</p>;
  if (!v) return <p>{t('gates.loading')}</p>;
  return (
    <section aria-label={t('gates.title')} data-testid="gates">
      <h2>{t('gates.title')}</h2>
      <div className="cards">
        <div className="card" data-testid="gate-native" data-status={v.native.status}><h3>{t('gates.native')}</h3><p>{label(v.native.status, KNOWN_GATE)}</p></div>
        <div className="card" data-testid="gate-improvement" data-status={v.improvement.status}>
          <h3>{t('gates.improvement')}</h3><p>{label(v.improvement.status, KNOWN_GATE)}{v.improvement.reason_code ? ` · ${v.improvement.reason_code}` : ''}</p>
        </div>
        <div className="card" data-testid="gate-combined"><h3>{t('gates.combined')}</h3><p>{v.combined.decision}{v.combined.reason_code ? ` · ${v.combined.reason_code}` : ''}</p></div>
      </div>
    </section>
  );
}

export function DiffView() {
  const { v, err } = useLoad<DiffT>(() => api.diff(), 'diff');
  if (err) return <p>{t('diff.unavailable')}</p>;
  if (!v) return <p>{t('diff.loading')}</p>;
  return (
    <section aria-label={t('diff.section')}>
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
