import type { z } from 'zod';
import type * as S from '../api/schemas';
import { t, ES_419, type I18nKey } from '../i18n/es419';
import { attemptOutcome, type Hyp } from './demoModel';

type Alt = z.infer<typeof S.Alternatives>['items'][number];
type Attempt = z.infer<typeof S.GateAttempt>;
type Ref = { id: string; digest: string; media_type: string } | null | undefined;

const num = (v: number | null | undefined, pct = false) => (v === null || v === undefined ? 'unknown' : pct ? `${(v * 100).toFixed(2)}%` : String(v));
/** Machine ids of doubles map to honest es-419 labels; an id with no label is shown verbatim, never hidden. */
export const doubleLabel = (id: string) => {
  const k = `double.${id}` as I18nKey;
  return k in ES_419 ? t(k) : id;
};

export function AlternativesPanel({ items }: { items: Alt[] }) {
  if (items.length === 0) return null;
  return (
    <section aria-label={t('alt.title')} data-testid="alternatives">
      <h2>{t('alt.title')}</h2>
      <table className="alts">
        <caption>{t('alt.caption')}</caption>
        <thead><tr>
          <th scope="col">{t('alt.option')}</th><th scope="col">{t('alt.summary')}</th>
          <th scope="col">{t('alt.abandoned')}</th><th scope="col">{t('alt.risk')}</th>
        </tr></thead>
        <tbody>
          {items.map((a) => (
            <tr key={a.id} data-testid={`alt-${a.id}`} data-kind={a.kind}>
              <th scope="row">{a.kind === 'do_nothing' ? t('alt.doNothing') : a.kind === 'proposed_change' ? t('alt.proposed') : a.kind}</th>
              <td>{a.summary}</td><td>{num(a.expected_abandoned)}</td><td>{a.risk ?? 'unknown'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

export function AttemptHistory({ attempts }: { attempts: Attempt[] }) {
  if (attempts.length === 0) return null;
  return (
    <section aria-label={t('attempts.title')} data-testid="attempt-history">
      <h3>{t('attempts.title')}</h3>
      <ol>
        {attempts.map((a) => {
          const outcome = attemptOutcome(a);
          const imp = a.improvement;
          return (
            <li key={a.attempt} className="attempt" data-testid={`attempt-${a.attempt}`} data-outcome={outcome}>
              <strong>{t('attempts.item', { n: a.attempt, id: a.candidate_id, scope: a.scope ?? 'unknown' })}</strong>
              {a.revision_of ? <> · <em>{t('attempts.revisionOf', { id: a.revision_of })}</em></> : null}
              <p>{t('attempts.native', { status: a.native })}{a.native_reason ? ` · ${a.native_reason}` : ''}</p>
              <p>{t('attempts.improvement', { status: imp.status })}{imp.reason_code ? ` · ${imp.reason_code}` : ''}</p>
              <p>{t('attempts.metrics', { lift: num(imp.lift, true), lo: num(imp.lift_lo, true), exp: num(imp.exposure, true), max: num(imp.guard_max_exposure, true) })}</p>
              <p><strong>{outcome === 'pass' ? t('attempts.outcomePass') : t('attempts.outcomeFail')}</strong></p>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

export function HypothesesList({ hypotheses }: { hypotheses: Hyp[] }) {
  if (hypotheses.length === 0) return null;
  return (
    <section aria-label={t('hyp.title')} data-testid="hypotheses">
      <h3>{t('hyp.title')}</h3>
      <ul>
        {hypotheses.map((h) => (
          <li key={h.id} className="hyp" data-testid={`hyp-${h.id}`} data-verdict={h.verdict}>
            <strong>{h.id === 'main' ? t('hyp.main') : t('hyp.competing', { id: h.id })}</strong>{h.id === 'main' ? `: ${h.statement}` : null}
            <p>{t('hyp.verdict')}<strong>{h.verdict}</strong></p>
            {h.supports.length > 0 && <ul aria-label={t('hyp.supports')}>{h.supports.map((x, i) => <li key={i} className="supports">{x}</li>)}</ul>}
            {h.counter.length > 0 && <ul aria-label={t('hyp.counter')}>{h.counter.map((x, i) => <li key={i} className="contradicts">{t('hyp.counter')}: {x}</li>)}</ul>}
          </li>
        ))}
      </ul>
    </section>
  );
}

export function NativeReport({ reportRef }: { reportRef: Ref }) {
  return (
    <p data-testid="native-report">
      {reportRef ? t('gates.report', { id: reportRef.id, digest: reportRef.digest.slice(0, 12) }) : t('gates.noReport')}
    </p>
  );
}
