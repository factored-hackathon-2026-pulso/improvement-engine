import { useCallback, useEffect, useRef, useState } from 'react';
import type { z } from 'zod';
import { api, loadConfig, streamEvents } from '../api/client';
import { isStale } from '../api/reconnect';
import type * as S from '../api/schemas';
import { acceptRevision, applyEvent, initStream, type DebugEvent, type StreamState } from '../state/runStore';
import { createDebounced, needsSideRefresh, SIDE_REFRESH_DEBOUNCE_MS, type SideRefresh } from '../state/sideRefresh';
import { Drawer } from '../components/Drawer';
import { label, Investigation, RunOutcome, TracePanel } from './Panels';
import { hookState } from './demoModel';
import { DecisionPanel } from './DecisionPanel';
import { useAnnounce } from '../a11y/AnnounceContext';
import { diffNodes } from '../a11y/announcer';
import { t, type I18nKey } from '../i18n/es419';

type GraphT = z.infer<typeof S.Graph>;
type Conn = 'connecting' | 'live' | 'stale' | 'reconnecting' | 'session_expired' | 'forbidden';
const KNOWN_NODE = ['planned', 'queued', 'running', 'complete', 'dead', 'cancelled', 'superseded', 'retry_wait', 'deferred', 'waiting_dependency', 'unknown'];
const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;
const MAX_NODE_ANNOUNCEMENTS = 3;

export function RunView({ runId, nodeId, onNode, onProfileChanged }: { runId: string; nodeId: string | null; onNode: (id: string | null) => void; onProfileChanged?: () => void }) {
  const announce = useAnnounce();
  const [graph, setGraph] = useState<GraphT | null>(null);
  const [failed, setFailed] = useState(false);
  const [pulse, setPulse] = useState<string[]>([]);
  const [purged, setPurged] = useState<{ floor: number | null } | null>(null);
  const [gatesTick, setGatesTick] = useState(0);
  const [conn, setConnState] = useState<Conn>('connecting');
  const rev = useRef(-1);
  const prevNodes = useRef<GraphT['nodes'] | null>(null);
  const stream = useRef<StreamState>(initStream(0));
  const floor = useRef(0);
  const ready = useRef(false);
  const early = useRef<DebugEvent[]>([]);
  const connRef = useRef<Conn>('connecting');
  const lastSignal = useRef<number | null>(null);
  const heartbeatMs = useRef(5000);

  const setConn = useCallback((c: Conn) => { connRef.current = c; setConnState(c); }, []);

  const reload = useCallback(async () => {
    const g = await api.graph(runId);
    if (!acceptRevision(rev.current, g.projection_revision)) return;
    rev.current = g.projection_revision;
    // Announce only the nodes that changed (rate limited by the single announcer), never the whole graph.
    const changed = diffNodes(prevNodes.current, g.nodes);
    prevNodes.current = g.nodes;
    if (changed.length > MAX_NODE_ANNOUNCEMENTS) announce(t('announce.nodes', { n: changed.length }));
    else changed.forEach((c) => announce(t('announce.node', { label: c.label, status: c.status })));
    setGraph(g);
  }, [runId, announce]);

  // Animation bus: only live, deduped, ordered events pulse; the timer only clears a visual flag.
  const animate = useCallback((events: DebugEvent[]) => {
    if (reduced()) return;
    const ids = events.filter((e) => e.entity_ref.kind === 'node').map((e) => e.entity_ref.id);
    setPulse((p) => [...p, ...ids]);
    setTimeout(() => setPulse((p) => p.filter((x) => !ids.includes(x))), 1000);
  }, []);

  // gates_set / doubles_declared: refetch the profile (banner) and the gates, debounced (one refetch per burst).
  const pending = useRef<SideRefresh>({ profile: false, gates: false });
  const onProfileRef = useRef(onProfileChanged);
  onProfileRef.current = onProfileChanged;
  const sideRefresh = useRef(createDebounced(() => {
    const p = pending.current; pending.current = { profile: false, gates: false };
    if (p.profile) onProfileRef.current?.();
    if (p.gates) setGatesTick((n) => n + 1);
  }, SIDE_REFRESH_DEBOUNCE_MS)).current;
  useEffect(() => () => sideRefresh.cancel(), [sideRefresh]);
  const noteSide = useCallback((events: DebugEvent[]) => {
    const n = needsSideRefresh(events);
    if (!n.profile && !n.gates) return;
    pending.current = { profile: pending.current.profile || n.profile, gates: pending.current.gates || n.gates };
    sideRefresh();
  }, [sideRefresh]);

  const handle = useCallback((ev: DebugEvent, live: boolean) => {
    const r = applyEvent(stream.current, ev);
    stream.current = r.state;
    if (r.effect.type === 'apply') { void reload(); noteSide(r.effect.events); if (live) animate(r.effect.events); }
    else if (r.effect.type === 'catchup') {
      void api.events(runId, r.effect.after_sequence).then((p) => p.items.forEach((e) => handle(e, false)));
    }
  }, [animate, reload, runId, noteSide]);

  useEffect(() => { void loadConfig().then((c) => { heartbeatMs.current = c.sseHeartbeatMs; }); }, []);

  useEffect(() => {
    rev.current = -1; stream.current = initStream(0); floor.current = 0; ready.current = false; early.current = [];
    prevNodes.current = null; lastSignal.current = null;
    setGraph(null); setFailed(false); setPurged(null); setConn('connecting');
    const baseline = () => Promise.all([reload(), api.events(runId, floor.current)]).then(([, p]) => {
      stream.current = initStream(p.items.reduce((m, e) => Math.max(m, e.sequence), floor.current));
      ready.current = true;
      early.current.forEach((e) => handle(e, true));
      early.current = [];
    }).catch(() => setFailed(true));
    const stop = streamEvents(runId, {
      onEvent: (e) => { if (ready.current) handle(e, true); else early.current.push(e); },
      onActivity: () => {
        lastSignal.current = Date.now();
        if (connRef.current === 'stale') setConn('live');
      },
      onOpen: (reconnect) => {
        setConn('live');
        if (!reconnect) { void baseline(); return; }
        // Resume: re-read the projection and catch up from the last applied sequence (no live animation).
        void reload();
        void api.events(runId, stream.current.lastSeq).then((p) => p.items.forEach((e) => handle(e, false)));
      },
      onDrop: () => setConn('reconnecting'),
      onResnapshot: ({ recoveryCursor }) => {
        // 410: snapshot + floor = recovery_cursor; earlier history is gone and is never invented.
        floor.current = recoveryCursor ?? 0;
        ready.current = false; early.current = []; stream.current = initStream(floor.current); rev.current = -1;
        setPurged({ floor: recoveryCursor });
        setConn('reconnecting');
      },
      onFatal: (a) => setConn(a),
    }, { lastEventId: () => stream.current.lastSeq });
    // Silence is not completion: no event and no heartbeat for 2x the heartbeat interval marks the view stale.
    const tick = setInterval(() => {
      if (connRef.current === 'live' && isStale(lastSignal.current, Date.now(), heartbeatMs.current)) setConn('stale');
    }, 250);
    return () => { stop(); clearInterval(tick); };
  }, [runId, handle, reload, setConn]);

  const prevConn = useRef<Conn>('connecting');
  useEffect(() => {
    const was = prevConn.current;
    prevConn.current = conn;
    if (conn === 'connecting') return;
    if (conn === 'live' && (was === 'connecting')) return; // first connection is not news
    announce(t(`stream.${conn}` as I18nKey));
  }, [conn, announce]);
  useEffect(() => { if (purged) announce(t('history.purged', { floor: purged.floor === null ? '' : t('history.floor', { seq: purged.floor }) })); }, [purged, announce]);
  useEffect(() => { if (failed) announce(t('run.failed')); }, [failed, announce]);

  if (failed) return <p data-testid="run-failed">{t('run.failed')}</p>;
  if (!graph) return <p>{t('run.loadingGraph')}</p>;
  const selected = graph.nodes.find((n) => n.node_id === nodeId) ?? null;
  const stale = conn === 'stale' || conn === 'reconnecting' || conn === 'session_expired';
  return (
    <div>
      <h1>{t('run.title', { id: runId })} <small>({graph.status})</small></h1>
      <p data-testid="stream-status" data-state={conn}>{t(`stream.${conn}` as I18nKey)}</p>
      {purged && (
        <p data-testid="history-purged" className="banner warn">
          {t('history.purged', { floor: purged.floor === null ? '' : t('history.floor', { seq: purged.floor }) })}
        </p>
      )}
      <div className="graph" role="group" aria-label={t('run.graphGroup')} data-stale={stale ? '1' : '0'}>
        {graph.nodes.map((n) => (
          <button
            key={n.node_id} type="button" className="node" data-testid={`node-${n.node_id}`}
            data-status={n.status} data-pulse={pulse.includes(n.node_id) ? '1' : '0'} data-hook={hookState(n) ?? undefined}
            onClick={() => onNode(n.node_id)}
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
            {hookState(n) && <span className="hook"> {t(`hook.${hookState(n)!}` as I18nKey)}</span>}
          </li>
        ))}
      </ol>
      {selected && (
        <Drawer key={selected.node_id} title={t('drawer.node', { label: selected.label })} instance={selected.node_id} onClose={() => onNode(null)}>
          <p>{t('drawer.status', { value: label(selected.status, KNOWN_NODE) })}</p>
          <p>{t('drawer.stage', { value: selected.stage })}</p>
          <p>{t('drawer.reason', { value: selected.reason_code ?? t('drawer.noReason') })}</p>
          {hookState(selected) && <p className="hook">{t(`hook.${hookState(selected)!}` as I18nKey)}</p>}
        </Drawer>
      )}
      <TracePanel nodes={graph.nodes} />
      <Investigation runId={runId} refresh={gatesTick} />
      <RunOutcome runId={runId} refresh={gatesTick} />
      <DecisionPanel runId={runId} refresh={gatesTick} hookPending={graph.nodes.some((n) => hookState(n) === 'decision_pending')} />
    </div>
  );
}
