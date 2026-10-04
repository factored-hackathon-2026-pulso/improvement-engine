import { useEffect, useRef, useState } from 'react';
import { t, type I18nKey } from '../../i18n/es419';
import { httpAutomationApi, type AutomationApi } from './api';
import type { CaseType, CaseTypeDetail, CaseTypeList, Metric } from './schemas';
import './automation.css';

const K = (k: string) => k as I18nKey;
const per = (n?: number, d?: number, of = 10) => (d ? Math.round(((n ?? 0) / d) * of) : 0);
const fmtDate = (iso: string | null) => (iso ? new Date(`${iso}T12:00:00Z`).toLocaleDateString('es-419', { day: 'numeric', month: 'long', timeZone: 'UTC' }) : '');
const stageNum = (s: CaseType['stage']) => (s === 'agent' ? 4 : typeof s === 'number' ? s : 0);
const reasonText = (r?: string) => {
  const key = `auto.reason.${r ?? 'no_input'}`;
  return key in AUTO_KEYS ? t(K(key)) : t('auto.reason.other', { reason: r ?? '' });
};
const AUTO_KEYS: Record<string, true> = {
  'auto.reason.no_draft_rows': true, 'auto.reason.no_input': true, 'auto.reason.no_agent_run': true,
  'auto.reason.no_tool_use_events': true, 'auto.reason.no_copilot_query_signature': true,
};

export function SourceBadge({ source, simulated }: { source?: string; simulated?: boolean }) {
  if (!source) return null;
  return (
    <span className="auto-badge" data-testid="source-badge" data-source={source} data-simulated={simulated ? '1' : '0'}>
      {t(K(`auto.src.${source}`))}{simulated ? <> · <strong>{t('auto.simulatedTag')}</strong></> : null}
    </span>
  );
}

export function StageBar({ stage }: { stage: CaseType['stage'] }) {
  const n = stageNum(stage);
  const filled = Math.min(n, 3);
  return (
    <span className="auto-bar" data-stage={String(stage)} aria-hidden="true">
      {[1, 2, 3].map((i) => <i key={i} className={i <= filled ? (n === 4 ? 'on agent' : 'on') : ''} />)}
    </span>
  );
}

function measureText(c: CaseType): string {
  const m = c.measure;
  switch (m.kind) {
    case 'draft_accept_100': return t('auto.measure.draft_accept_100', { n: m.numerator ?? 0 });
    case 'agent_resolved': {
      const r = m.numerator ?? 0; const h = m.denominator ?? 0;
      return m.handed == null ? t('auto.measure.agent_resolved_nohand', { r, h }) : t('auto.measure.agent_resolved', { r, h, x: m.handed });
    }
    case 'tool_use_rate': return t('auto.measure.tool_use_rate', { n: per(m.numerator, m.denominator) });
    case 'copilot_use': return t('auto.measure.copilot_use', { n: per(m.numerator, m.denominator) });
    case 'repeat_q': return t('auto.measure.repeat_q', { n: m.numerator ?? 0 });
    default: return t('auto.measure.none');
  }
}

function stageLabel(c: CaseType): string {
  return c.stage === 'agent' ? t('auto.withAgent') : t('auto.stageN', { n: stageNum(c.stage) });
}

function MetricToday({ m, text }: { m: Metric; text: string }) {
  return m.status === 'ok' ? <>{text}</> : <>{t('auto.notComputable', { reason: reasonText(m.reason) })}</>;
}

function Drawer({ id, api, onClose, onProposal }: { id: string; api: AutomationApi; onClose: () => void; onProposal: (id: string) => void }) {
  const [d, setD] = useState<CaseTypeDetail | null>(null);
  const [failed, setFailed] = useState(false);
  const ref = useRef<HTMLElement>(null);
  const opener = useRef<Element | null>(document.activeElement);
  useEffect(() => { let live = true; api.caseType(id).then((x) => live && setD(x)).catch(() => live && setFailed(true)); return () => { live = false; }; }, [api, id]);
  useEffect(() => {
    ref.current?.focus();
    const back = opener.current;
    return () => { if (back instanceof HTMLElement) back.focus(); };
  }, []);
  const title = d?.label ?? id;
  const steps = ['1', '2', '3', 'agent'];
  return (
    <aside ref={ref} tabIndex={-1} role="dialog" aria-label={title} className="auto-drawer" data-testid="auto-drawer" onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } }}>
      <header>
        <div>
          <h2>{title}</h2>
          {d && <div className="auto-stagebar"><StageBar stage={d.stage} /> <span>{stageLabel(d)}</span></div>}
        </div>
        <button type="button" className="auto-x" aria-label={t('auto.drawerClose')} onClick={onClose}>×</button>
      </header>
      {failed && <p role="alert">{t('auto.unavailable')}</p>}
      {!d && !failed && <p>{t('auto.loading')}</p>}
      {d && (
        <div className="auto-drawer-body">
          <h3>{t('auto.howMatured')}</h3>
          <ol className="auto-steps">
            {steps.map((k) => {
              const h = d.history.find((x) => x.stage === k);
              const proposedToday = k === 'agent' && !h && d.history.some((x) => x.stage === 'agent_proposed');
              if (!h && !proposedToday) return null;
              return (
                <li key={k} data-step={k}>
                  <span className="auto-dot" aria-hidden="true">{k === 'agent' ? '✓' : k}</span>
                  <b>{t(K(k === 'agent' ? 'auto.stepAgent' : `auto.step${k}`))}</b>
                  <small>{proposedToday ? t('auto.proposedToday') : t('auto.since', { date: fmtDate(h?.since ?? null) })}</small>
                </li>
              );
            })}
          </ol>
          <h3>{t('auto.draftsTitle')}</h3>
          {d.drafts_last_100.status === 'ok' ? (
            <div data-testid="auto-drafts">
              <div className="auto-split" role="img" aria-label={t('auto.draftsTitle')}>
                <i className="a" style={{ flex: d.drafts_last_100.as_is }} /><i className="m" style={{ flex: d.drafts_last_100.minor }} /><i className="x" style={{ flex: d.drafts_last_100.discarded }} />
              </div>
              <p className="auto-legend2"><span>{t('auto.draftsAsIs', { n: d.drafts_last_100.as_is ?? 0 })}</span><span>{t('auto.draftsMinor', { n: d.drafts_last_100.minor ?? 0 })}</span><span>{t('auto.draftsDiscarded', { n: d.drafts_last_100.discarded ?? 0 })}</span></p>
              <SourceBadge source={d.drafts_last_100.source} simulated={d.drafts_last_100.simulated} />
            </div>
          ) : <p data-testid="auto-drafts">{t('auto.notComputable', { reason: reasonText(d.drafts_last_100.reason) })}</p>}
          <h3>{t('auto.thresholdsTitle')}</h3>
          <ul className="auto-th">
            {d.thresholds_today.map((r) => {
              const today = r.today;
              const k = r.key === 'stage1_to_2' ? '1to2' : r.key === 'stage2_to_3' ? '2to3' : '3toAgent';
              const rule = r.key === 'stage1_to_2' ? t('auto.th1to2Rule', { min: r.min }) : r.key === 'stage2_to_3' ? t('auto.th2to3Rule', { min: Math.round(r.min * 10) }) : t('auto.th3toAgentRule', { min: Math.round(r.min * 10), w: r.window ?? 100 });
              const now = r.key === 'stage1_to_2' ? t('auto.th1to2Today', { n: today.numerator ?? 0 }) : r.key === 'stage2_to_3' ? t('auto.th2to3Today', { n: per(today.numerator, today.denominator) }) : t('auto.th3toAgentToday', { n: today.numerator ?? 0, d: today.denominator ?? 0 });
              return (
                <li key={r.key} data-met={r.met ? '1' : '0'} data-testid={`auto-th-${r.key}`}>
                  <b>{t(K(`auto.th${k}`))}</b>
                  <span>{rule}</span>
                  <span><MetricToday m={today} text={now} /> <SourceBadge source={today.source} simulated={today.simulated} /></span>
                  <small>{r.met ? t('auto.thMet') : t('auto.thNotMet')}</small>
                </li>
              );
            })}
          </ul>
          <p className="auto-note">{t('auto.drawerNote')}</p>
        </div>
      )}
      <footer>
        <button type="button" className="auto-btn dark" disabled={!d?.proposal} onClick={() => onProposal(id)}>{t('auto.viewProposal')}</button>
      </footer>
    </aside>
  );
}

function ProposalView({ id, api, onBack }: { id: string; api: AutomationApi; onBack: () => void }) {
  const [d, setD] = useState<CaseTypeDetail | null>(null);
  const [state, setState] = useState<string>('proposed');
  const [err, setErr] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => { let live = true; api.caseType(id).then((x) => { if (live) { setD(x); setState(x.proposal?.state ?? 'proposed'); } }).catch(() => live && setErr(true)); return () => { live = false; }; }, [api, id]);
  const p = d?.proposal ?? null;
  const act = async (f: () => Promise<{ state: string }>) => {
    setBusy(true); setErr(false);
    try { setState((await f()).state); } catch { setErr(true); } finally { setBusy(false); }
  };
  return (
    <section className="auto-proposal" data-testid="auto-proposal" aria-labelledby="auto-prop-h">
      <button type="button" className="auto-btn light" onClick={onBack}>{t('auto.back')}</button>
      <h2 id="auto-prop-h">{t('auto.propTitle', { type: d?.label ?? id })}</h2>
      <p className="auto-warn">{t('auto.propSimulated')}</p>
      {!d && !err && <p>{t('auto.loading')}</p>}
      {d && !p && <p>{t('auto.noProposal')}</p>}
      {p && (
        <>
          <dl>
            <div><dt>{t('auto.propId', { id: p.proposal_id })}</dt><dd><code>{p.proposal_id}</code></dd></div>
            <div><dt>{t('auto.propTarget', { target: `${p.target.agent_id}@${p.target.alias}` })}</dt></div>
            <div><dt>{t('auto.propArtifact', { from: p.target.artifact, to: p.target.candidate_artifact ?? p.target.artifact })}</dt></div>
            {p.target.eval_suite && <div><dt>{t('auto.propEval', { suite: p.target.eval_suite })}</dt></div>}
            {p.related_agents && <div><dt>{t('auto.propRelated', { agents: p.related_agents.join(', ') })}</dt></div>}
            {p.origin && <div><dt>{t('auto.propOrigin', { origin: p.origin })}</dt></div>}
            {p.alternative_target && <div><dt>{t('auto.propAlt', { target: `${p.alternative_target.agent_id}@${p.alternative_target.alias}`, artifact: p.alternative_target.artifact })}</dt></div>}
            <div><dt>{t('auto.propHash', { hash: p.candidate_hash })}</dt></div>
            <div><dt>{t('auto.propVerdict', { verdict: p.ledger_verdict })}</dt></div>
            <div><dt>{t('auto.propGrade', { grade: p.link_grade })}</dt></div>
          </dl>
          <p><a href={`#/run/${p.run_id}`}>{t('auto.propRunLink')}</a></p>
          <p data-testid="auto-prop-state" data-state={state}><b>{t('auto.propState')}</b>: {t(K(`auto.propState.${state}`))}</p>
          <p className="auto-note">{t('auto.approveNote')}</p>
          <div className="auto-actions">
            <button type="button" className="auto-btn dark" disabled={busy || state !== 'proposed'} onClick={() => void act(() => api.approve(id, p.candidate_hash))}>{t('auto.approve')}</button>
            <button type="button" className="auto-btn light" disabled={busy || state !== 'approved_simulated'} onClick={() => void act(() => api.publishStaging(id))}>{t('auto.publish')}</button>
          </div>
        </>
      )}
      {err && <p role="alert">{t('auto.actionFailed')}</p>}
    </section>
  );
}

export function AutomationView({ api = httpAutomationApi }: { api?: AutomationApi }) {
  const [data, setData] = useState<CaseTypeList | null>(null);
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [proposal, setProposal] = useState<string | null>(null);
  useEffect(() => { let live = true; api.caseTypes().then((x) => live && setData(x)).catch(() => live && setFailed(true)); return () => { live = false; }; }, [api]);
  const scope = [...new Set((data?.doubles ?? []).flatMap((x) => x.scope))].map((s) => (`auto.scope.${s}` in { ...AUTO_SCOPES } ? t(K(`auto.scope.${s}`)) : s)).join(', ');
  const banner = data?.banner ?? null;
  return (
    <div className="auto" data-testid="auto-page">
      <div className="auto-top">
        <span>{t('auto.crumb')}</span>
        <span className="auto-tabs"><b aria-current="page">{t('auto.tabTypes')}</b><span aria-disabled="true">{t('auto.tabAgents')}</span></span>
      </div>
      <div className="auto-main">
        <h1>{t('auto.title')}</h1>
        <p className="auto-intro">{t('auto.intro')}</p>
        {data && <p className="auto-sim" role="note" data-testid="auto-simulated-notice">{data.doubles.length ? t('auto.simNotice', { scope }) : t('auto.simNoticeNone')}</p>}
        {data && data.assumptions.length > 0 && <ul className="auto-sim" data-testid="auto-assumptions">{data.assumptions.map((a) => <li key={a}>{`auto.assume.${a}` in ASSUME ? t(K(`auto.assume.${a}`)) : a}</li>)}</ul>}
        {failed && <p role="alert">{t('auto.unavailable')}</p>}
        {!data && !failed && <p>{t('auto.loading')}</p>}
        {data && proposal && <ProposalView id={proposal} api={api} onBack={() => setProposal(null)} />}
        {data && !proposal && (
          <>
            {banner && (
              <section className="auto-banner" data-testid="auto-banner" aria-label={t('auto.bannerTitle', { type: banner.label ?? banner.type_id })}>
                <span className="auto-wand" aria-hidden="true">✦</span>
                <div>
                  <b>{t('auto.bannerTitle', { type: banner.label ?? banner.type_id })}</b>
                  <p>{t('auto.bannerText', { n: per(banner.numerator ?? 0, banner.denominator ?? 0) })} <SourceBadge source="sim_draft_stream" simulated={banner.simulated} /></p>
                </div>
                <button type="button" className="auto-btn dark" onClick={() => setProposal(banner.type_id)}>{t('auto.bannerButton')}</button>
              </section>
            )}
            <section className="auto-card" aria-labelledby="auto-list-h">
              <header>
                <h2 id="auto-list-h">{t('auto.listTitle')}</h2>
                <ul className="auto-legend" aria-hidden="true">
                  {([0, 1, 2, 3] as const).map((n) => <li key={n}><StageBar stage={n} /> {t(K(`auto.legend${n}`))}</li>)}
                  <li><StageBar stage="agent" /> {t('auto.legendAgent')}</li>
                </ul>
              </header>
              <table>
                <thead><tr><th scope="col">{t('auto.colType')}</th><th scope="col">{t('auto.colStage')}</th><th scope="col">{t('auto.colMeasure')}</th><th scope="col">{t('auto.colCases')}</th><td /></tr></thead>
                <tbody>
                  {data.case_types.map((c) => (
                    <tr key={c.type_id} data-testid={`auto-row-${c.type_id}`} data-stage={String(c.stage)} data-selected={open === c.type_id ? '1' : '0'}>
                      <th scope="row"><b>{c.label ?? c.type_id}</b><small>{c.group}</small></th>
                      <td><span className="auto-stagebar"><StageBar stage={c.stage} /> <span>{stageLabel(c)}</span></span></td>
                      <td data-testid={`auto-measure-${c.type_id}`}>{measureText(c)} <SourceBadge source={c.measure.source} simulated={c.measure.simulated} /></td>
                      <td>{c.cases_today ?? ''}</td>
                      <td>
                        <button type="button" className={c.agent_proposed ? 'auto-btn blue' : 'auto-btn light'} onClick={() => setOpen(c.type_id)}>
                          {c.agent_proposed ? t('auto.agentProposed') : t('auto.viewSignals')}<span className="sr-only"> {c.label ?? c.type_id}</span>
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
            <p className="auto-foot">{t('auto.footer')}</p>
          </>
        )}
      </div>
      {open && <Drawer id={open} api={api} onClose={() => setOpen(null)} onProposal={(i) => { setOpen(null); setProposal(i); }} />}
    </div>
  );
}

const AUTO_SCOPES = {
  'auto.scope.draft_dispositions': 1, 'auto.scope.case_types': 1, 'auto.scope.stage_history': 1, 'auto.scope.cases_today': 1,
  'auto.scope.agent_runs': 1, 'auto.scope.approval': 1, 'auto.scope.publish_staging': 1,
};

const ASSUME: Record<string, true> = { 'auto.assume.e0_disputar_cargo_as_cobro_indebido': true };
