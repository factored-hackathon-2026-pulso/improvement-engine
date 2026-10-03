import { CAPABILITIES, DEMO_SOURCES, eligibility, insightsOf, type Source } from './sourcesModel';
import { tl, type Locale, type SourcesKey } from '../i18n/sources';

const K = (k: string) => k as SourcesKey;

function SourceSection({ s, locale }: { s: Source; locale: Locale }) {
  const L = (k: SourcesKey, p?: Record<string, string | number>) => tl(locale, k, p);
  const hid = `src-h-${s.id}`;
  const insights = insightsOf(s);
  return (
    <section aria-labelledby={hid} data-testid={`source-${s.id}`} className="source">
      <h2 id={hid}>{L(K(`sources.src.${s.id}`))} <small><code>{s.id}</code></small></h2>
      <p>{L('sources.phase', { value: s.phase === null ? L('sources.noPhase') : `${s.phase}` })} · <code>{s.profileVersion}</code></p>
      <p>{L('sources.channels', { value: s.channels.join(', ') })}</p>
      <table className="alts">
        <caption>{L('sources.capsCaption')}</caption>
        <thead><tr><th scope="col">{L('sources.capsHead')}</th><th scope="col">{L('sources.presence')}</th></tr></thead>
        <tbody>
          {CAPABILITIES.map((c) => (
            <tr key={c} data-testid={`cap-${s.id}-${c}`} data-present={s.capabilities[c] ? '1' : '0'}>
              <th scope="row">{L(K(`sources.cap.${c}`))} <small><code>{c}</code></small></th>
              <td>{s.capabilities[c] ? L('sources.present') : L('sources.absent')}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <table className="alts">
        <caption>{L('sources.famCaption')}</caption>
        <thead><tr><th scope="col">{L('sources.famHead')}</th><th scope="col">{L('sources.famStatus')}</th><th scope="col">{L('sources.famMissing')}</th></tr></thead>
        <tbody>
          {eligibility(s).map((e) => (
            <tr key={e.family} data-testid={`fam-${s.id}-${e.family}`} data-status={e.status}>
              <th scope="row">{L(K(`sources.fam.${e.family}`))} <small><code>{e.family}</code></small></th>
              <td>{L(K(`sources.${e.status}`))} <code>{e.status}</code></td>
              <td>{e.missing.length === 0 ? (e.status === 'eligible' ? L('sources.none') : L('sources.noTimeline')) : e.missing.map((m) => <code key={m}>{m} </code>)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div data-testid={`counters-${s.id}`}>
        <h3>{L('sources.countersTitle')}</h3>
        {s.counters === null ? <p>{L('sources.noExporter')}</p> : (
          <dl>
            {(Object.keys(s.counters) as (keyof NonNullable<Source['counters']>)[]).map((k) => (
              <div key={k} data-testid={`counter-${k}`}><dt>{L(K(`sources.counter.${k}`))} <small><code>{k}</code></small></dt><dd>{s.counters![k]}</dd></div>
            ))}
          </dl>
        )}
        {s.teamGeneratedExcluded !== null && <p data-testid="team-generated">{L('sources.teamGen', { n: s.teamGeneratedExcluded })}</p>}
      </div>
      {s.id === 'platform_live' && (
        <div>
          <h3>{L('sources.insightsTitle')}</h3>
          <p>{L('sources.insightsNote')}</p>
          {insights.length === 0 ? <p>{L('sources.noInsights')}</p> : (
            <ul className="cards">
              {insights.map((i) => (
                <li key={i.id} className="card" data-testid={`insight-${i.id}`} data-status={i.status}>
                  <strong>{L(K(`sources.fam.${i.family}`))}</strong>
                  <p>{L('sources.insightStatus', { value: i.status })}</p>
                  <p>{L('sources.insightReason', { value: i.reason })}</p>
                  <p>{L('sources.insightDen', { n: i.denominator })}</p>
                  <p>{L('sources.insightProv', { value: i.provenance })}</p>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}

/** Read-only backoffice section: no mutating actions exist here by construction. Data is a stand_in demo model. */
export function SourcesView({ locale = 'es-419' }: { locale?: Locale }) {
  return (
    <>
      <h1>{tl(locale, 'sources.title')}</h1>
      <div className="banner stand_in" data-testid="sources-standin" data-level="stand_in">{tl(locale, 'sources.standIn')}</div>
      {DEMO_SOURCES.map((s) => <SourceSection key={s.id} s={s} locale={locale} />)}
    </>
  );
}
