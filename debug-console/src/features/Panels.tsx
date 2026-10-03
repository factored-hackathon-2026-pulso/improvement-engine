import { useEffect, useState } from 'react';
import type { z } from 'zod';
import { api } from '../api/client';
import type * as S from '../api/schemas';

type Inv = z.infer<typeof S.Investigation>;
type GatesT = z.infer<typeof S.Gates>;
type DiffT = z.infer<typeof S.Diff>;
type MemT = z.infer<typeof S.Memory>;

const KNOWN_GATE = ['pending', 'pass', 'fail', 'unknown', 'not_evaluable', 'insufficient_power', 'dependency_blocked', 'unsupported', 'failed_infra'];
export const label = (v: string, known: string[]) => (known.includes(v) ? v : `unrecognised: ${v}`);

function useLoad<T>(fn: () => Promise<T>, dep: string) {
  const [v, setV] = useState<T | null>(null);
  const [err, setErr] = useState(false);
  useEffect(() => { setV(null); setErr(false); fn().then(setV).catch(() => setErr(true)); }, [dep]); // eslint-disable-line react-hooks/exhaustive-deps
  return { v, err };
}

const RELATIONS: [string, string][] = [['contradicts', 'Contraevidencia'], ['supports', 'Evidencia a favor'], ['limits', 'Limitaciones']];

export function Investigation({ runId }: { runId: string }) {
  const { v, err } = useLoad<Inv>(() => api.investigation(runId), runId);
  if (err) return <p>unknown · investigación no disponible</p>;
  if (!v) return <p>Cargando investigación…</p>;
  return (
    <section aria-label="Investigación" data-testid="investigation">
      <h2>Investigación</h2>
      <p>Hipótesis: {v.hypothesis ?? 'unknown · sin hipótesis'}. Verificador: <strong data-testid="verifier">{v.verifier}</strong></p>
      {RELATIONS.map(([rel, title]) => (
        <div key={rel} data-testid={`ev-${rel}`}>
          <h3>{title} ({v.evidence.filter((e) => e.relation === rel).length})</h3>
          <ul>
            {v.evidence.filter((e) => e.relation === rel).map((e) => (
              <li key={e.evidence_ref.id} className={rel}>{title}: {e.summary} <small>({e.source_kind}, {e.validation})</small></li>
            ))}
          </ul>
        </div>
      ))}
      {v.evidence.some((e) => !RELATIONS.some(([r]) => r === e.relation)) && <p>Hay evidencia con relación no reconocida.</p>}
    </section>
  );
}

export function Gates({ runId }: { runId: string }) {
  const { v, err } = useLoad<GatesT>(() => api.gates(runId), runId);
  if (err) return <p>unknown · gates no disponibles</p>;
  if (!v) return <p>Cargando gates…</p>;
  return (
    <section aria-label="Gates" data-testid="gates">
      <h2>Gates</h2>
      <div className="cards">
        <div className="card" data-testid="gate-native" data-status={v.native.status}><h3>Gate nativo</h3><p>{label(v.native.status, KNOWN_GATE)}</p></div>
        <div className="card" data-testid="gate-improvement" data-status={v.improvement.status}>
          <h3>Gate de mejora</h3><p>{label(v.improvement.status, KNOWN_GATE)}{v.improvement.reason_code ? ` · ${v.improvement.reason_code}` : ''}</p>
        </div>
        <div className="card" data-testid="gate-combined"><h3>Decisión combinada</h3><p>{v.combined.decision}{v.combined.reason_code ? ` · ${v.combined.reason_code}` : ''}</p></div>
      </div>
    </section>
  );
}

export function DiffView() {
  const { v, err } = useLoad<DiffT>(() => api.diff(), 'diff');
  if (err) return <p>unknown · diff no disponible</p>;
  if (!v) return <p>Cargando diff…</p>;
  return (
    <section aria-label="Diff de la propuesta">
      <h2>Diff</h2>
      <div role="group" aria-label="Líneas del diff" tabIndex={0}>
        {v.lines.map((l, i) => (
          <pre key={i} className={l.op}>{l.op === 'add' ? '+ ' : l.op === 'del' ? '- ' : '  '}{l.text}</pre>
        ))}
      </div>
    </section>
  );
}

export function MemoryView() {
  const { v, err } = useLoad<MemT>(() => api.memory(), 'mem');
  if (err) return <p>unknown · memoria no disponible</p>;
  if (!v) return <p>Cargando memoria…</p>;
  return (
    <section aria-label="Memoria">
      <h2>Memoria</h2>
      <ul>{v.items.map((m) => (
        <li key={m.memory_id} data-testid={`mem-${m.memory_id}`}>{m.revoked ? 'Olvidado (revocado): ' : ''}{m.title} <small>[{m.status}]</small></li>
      ))}</ul>
    </section>
  );
}
