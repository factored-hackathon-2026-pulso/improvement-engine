import { useEffect, useState } from 'react';
import type { z } from 'zod';
import { api, ApiError, loadConfig, onSessionExpired, setCsrf } from '../api/client';
import type * as S from '../api/schemas';
import { RunView } from '../features/RunView';
import { MemoryView } from '../features/Panels';
import { ModeBanner } from './ModeBanner';
import { LiveRegionProvider, useAnnounce } from '../a11y/AnnounceContext';
import { t } from '../i18n/es419';

type Profile = z.infer<typeof S.Profile>;
type Runs = z.infer<typeof S.RunList>;
type SessionState = 'loading' | 'ok' | 'expired' | 'unavailable';

function useHash() {
  const [h, setH] = useState(window.location.hash);
  useEffect(() => {
    const f = () => setH(window.location.hash);
    window.addEventListener('hashchange', f);
    return () => window.removeEventListener('hashchange', f);
  }, []);
  return h;
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

function Shell() {
  const hash = useHash();
  const announce = useAnnounce();
  const [profile, setProfile] = useState<Profile | null>(null);
  const [simulated, setSimulated] = useState(false);
  const [provider, setProvider] = useState('unknown');
  const [session, setSession] = useState<SessionState>('loading');
  useEffect(() => {
    // public/config.json is the only source of the client provider (read at runtime, not baked into the bundle).
    void loadConfig().then((c) => setProvider(c.provider));
    const off = onSessionExpired(() => setSession('expired'));
    api.session()
      .then((s) => { setCsrf(s.csrf_token); setSimulated(s.auth.simulated); setSession((cur) => (cur === 'expired' ? cur : 'ok')); })
      .catch((e: unknown) => { setCsrf(''); setSession(e instanceof ApiError && e.status === 401 ? 'expired' : 'unavailable'); });
    api.profile().then(setProfile).catch(() => setProfile(null));
    return off;
  }, []);
  useEffect(() => {
    if (session === 'expired') announce(t('session.expired'));
    if (session === 'unavailable') announce(t('session.unavailable'));
  }, [session, announce]);

  const [path = '', query = ''] = hash.replace(/^#/, '').split('?');
  const run = path.match(/^\/run\/([^/]+)$/);
  const node = new URLSearchParams(query).get('node');
  return (
    <>
      <ModeBanner profile={profile} simulated={simulated} provider={provider} />
      {(session === 'expired' || session === 'unavailable') && (
        <div className="banner bad" data-testid="session-banner" data-state={session}>
          {t(session === 'expired' ? 'session.expired' : 'session.unavailable')}
        </div>
      )}
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

export function App() {
  return <LiveRegionProvider><Shell /></LiveRegionProvider>;
}
