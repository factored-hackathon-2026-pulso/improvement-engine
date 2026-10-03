import { useEffect, useState } from 'react';
import type { z } from 'zod';
import { api, setCsrf } from '../api/client';
import type * as S from '../api/schemas';
import { RunView } from '../features/RunView';
import { MemoryView } from '../features/Panels';
import { t } from '../i18n/es419';

type Profile = z.infer<typeof S.Profile>;
type Runs = z.infer<typeof S.RunList>;

function useHash() {
  const [h, setH] = useState(window.location.hash);
  useEffect(() => {
    const f = () => setH(window.location.hash);
    window.addEventListener('hashchange', f);
    return () => window.removeEventListener('hashchange', f);
  }, []);
  return h;
}

function ModeBanner({ profile, simulated }: { profile: Profile | null; simulated: boolean }) {
  if (!profile) return <div className="banner bad" role="status">{t('banner.unverified')}</div>;
  const bad = profile.doubles.length > 0 && profile.target === 'real';
  return (
    <div className={bad ? 'banner bad' : 'banner'} data-testid="mode-banner">
      {t('banner.mode', { target: profile.target, profile: profile.runtime_profile, doubles: profile.doubles.join(', ') || t('banner.none') })}
      {simulated && t('banner.simulated')}
    </div>
  );
}

function RunList() {
  const [runs, setRuns] = useState<Runs | null>(null);
  const [err, setErr] = useState(false);
  useEffect(() => { api.runs().then(setRuns).catch(() => setErr(true)); }, []);
  if (err) return <p>{t('runs.error')}</p>;
  if (!runs) return <p>{t('runs.loading')}</p>;
  if (runs.items.length === 0) return <p>{t('runs.empty')}</p>;
  return (
    <ul aria-label={t('runs.label')}>
      {runs.items.map((r) => (
        <li key={r.run_id}><a href={`#/run/${r.run_id}`}>{r.title}</a> <small>[{r.state}, {r.origin}]</small></li>
      ))}
    </ul>
  );
}

export function App() {
  const hash = useHash();
  const [profile, setProfile] = useState<Profile | null>(null);
  const [simulated, setSimulated] = useState(false);
  useEffect(() => {
    api.session().then((s) => { setCsrf(s.csrf_token); setSimulated(s.auth.simulated); }).catch(() => undefined);
    api.profile().then(setProfile).catch(() => setProfile(null));
  }, []);

  const [path = '', query = ''] = hash.replace(/^#/, '').split('?');
  const run = path.match(/^\/run\/([^/]+)$/);
  const node = new URLSearchParams(query).get('node');
  return (
    <>
      <ModeBanner profile={profile} simulated={simulated} />
      <nav aria-label={t('nav.label')}><a href="#/">{t('nav.runs')}</a> · <a href="#/memory">{t('nav.memory')}</a></nav>
      <main>
        {run?.[1]
          ? <RunView
              runId={run[1]} nodeId={node}
              onNode={(id) => { window.location.hash = id ? `#/run/${run[1]}?node=${id}` : `#/run/${run[1]}`; }}
            />
          : path === '/memory' ? <MemoryView /> : <><h1>{t('app.title')}</h1><RunList /></>}
      </main>
    </>
  );
}
