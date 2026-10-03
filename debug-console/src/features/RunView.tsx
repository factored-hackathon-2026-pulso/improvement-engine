import { useCallback, useEffect, useRef, useState } from 'react';
import type { z } from 'zod';
import { api, streamEvents } from '../api/client';
import type * as S from '../api/schemas';
import { acceptRevision, applyEvent, initStream, type DebugEvent, type StreamState } from '../state/runStore';
import { Drawer } from '../components/Drawer';
import { label, Investigation, Gates, DiffView } from './Panels';
import { DecisionPanel } from './DecisionPanel';
import { t, type I18nKey } from '../i18n/es419';

type GraphT = z.infer<typeof S.Graph>;
const KNOWN_NODE = ['planned', 'queued', 'running', 'complete', 'dead', 'cancelled', 'superseded', 'retry_wait', 'deferred', 'waiting_dependency', 'unknown'];
const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

export function RunView({ runId, nodeId, onNode }: { runId: string; nodeId: string | null; onNode: (id: string | null) => void }) {
  const [graph, setGraph] = useState<GraphT | null>(null);
  const [failed, setFailed] = useState(false);
  const [pulse, setPulse] = useState<string[]>([]);
  const rev = useRef(-1);
  const stream = useRef<StreamState>(initStream(0));
  const ready = useRef(false);
  const early = useRef<DebugEvent[]>([]);

  const reload = useCallback(async () => {
    const g = await api.graph(runId);
    if (acceptRevision(rev.current, g.projection_revision)) { rev.current = g.projection_revision; setGraph(g); }
  }, [runId]);

  // Animation bus: only live, deduped, ordered events pulse; the timer only clears a visual flag.
  const animate = useCallback((events: DebugEvent[]) => {
    if (reduced()) return;
    const ids = events.filter((e) => e.entity_ref.kind === 'node').map((e) => e.entity_ref.id);
    setPulse((p) => [...p, ...ids]);
    setTimeout(() => setPulse((p) => p.filter((x) => !ids.includes(x))), 1000);
  }, []);

  const handle = useCallback((ev: DebugEvent, live: boolean) => {
    const r = applyEvent(stream.current, ev);
    stream.current = r.state;
    if (r.effect.type === 'apply') { void reload(); if (live) animate(r.effect.events); }
    else if (r.effect.type === 'catchup') {
      void api.events(runId, r.effect.after_sequence).then((p) => p.items.forEach((e) => handle(e, false)));
    }
  }, [animate, reload, runId]);

  const [conn, setConn] = useState<'connecting' | 'live' | 'reconnecting' | 'session_expired' | 'forbidden'>('connecting');

  useEffect(() => {
    rev.current = -1; stream.current = initStream(0); ready.current = false; early.current = [];
    setGraph(null); setFailed(false); setConn('connecting');
    const baseline = () => Promise.all([reload(), api.events(runId, 0)]).then(([, p]) => {
      stream.current = initStream(p.items.reduce((m, e) => Math.max(m, e.sequence), 0));
      ready.current = true;
      early.current.forEach((e) => handle(e, true));
      early.current = [];
    }).catch(() => setFailed(true));
    const stop = streamEvents(runId, {
      onEvent: (e) => { if (ready.current) handle(e, true); else early.current.push(e); },
      onOpen: (reconnect) => {
        setConn('live');
        if (!reconnect) { void baseline(); return; }
        // Resume: re-read the projection and catch up from the last applied sequence (no live animation).
        void reload();
        void api.events(runId, stream.current.lastSeq).then((p) => p.items.forEach((e) => handle(e, false)));
      },
      onDrop: () => setConn('reconnecting'),
      onResnapshot: () => { ready.current = false; early.current = []; stream.current = initStream(0); rev.current = -1; setConn('reconnecting'); },
      onFatal: (a) => setConn(a),
    }, { lastEventId: () => stream.current.lastSeq });
    return stop;
  }, [runId, handle, reload]);

  if (failed) return <p role="alert">{t('run.failed')}</p>;
  if (!graph) return <p>{t('run.loadingGraph')}</p>;
  const selected = graph.nodes.find((n) => n.node_id === nodeId) ?? null;
  return (
    <div>
      <h1>{t('run.title', { id: runId })} <small>({graph.status})</small></h1>
      <p role="status" data-testid="stream-status" data-state={conn}>{t(`stream.${conn}` as I18nKey)}</p>
      <div className="graph" role="group" aria-label={t('run.graphGroup')}>
        {graph.nodes.map((n) => (
          <button
            key={n.node_id} type="button" className="node" data-testid={`node-${n.node_id}`}
            data-status={n.status} data-pulse={pulse.includes(n.node_id) ? '1' : '0'} onClick={() => onNode(n.node_id)}
          >
            <strong>{n.label}</strong><br />{label(n.status, KNOWN_NODE)}
          </button>
        ))}
      </div>
      <ol aria-label={t('run.textGraph')}>
        {graph.nodes.map((n) => (
          <li key={n.node_id} data-testid={`textgraph-${n.node_id}`}>
            {n.label}: {label(n.status, KNOWN_NODE)}{n.reason_code ? ` (${n.reason_code})` : ''}
            {n.depends_on.length ? t('run.dependsOn', { ids: n.depends_on.join(', ') }) : ''}
          </li>
        ))}
      </ol>
      <div aria-live="polite" className="sr-only" style={{ position: 'absolute', left: -9999 }}>{graph.nodes.map((n) => `${n.label} ${n.status}`).join('. ')}</div>
      {selected && (
        <Drawer key={selected.node_id} title={t('drawer.node', { label: selected.label })} instance={selected.node_id} onClose={() => onNode(null)}>
          <p>{t('drawer.status', { value: label(selected.status, KNOWN_NODE) })}</p>
          <p>{t('drawer.stage', { value: selected.stage })}</p>
          <p>{t('drawer.reason', { value: selected.reason_code ?? t('drawer.noReason') })}</p>
        </Drawer>
      )}
      <Investigation runId={runId} />
      <Gates runId={runId} />
      <DiffView />
      <DecisionPanel />
    </div>
  );
}
