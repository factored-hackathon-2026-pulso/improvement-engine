import { useEffect, useState } from 'react';
import type { z } from 'zod';
import { api, ApiError, loadConfig, onSessionExpired, reportSessionExpired, setCsrf } from '../api/client';
import { bootDebugApi, type DebugApi } from '../api/debug';
import { DebugApiProvider } from '../api/debug/context';
import { RunList } from '../features/RunList';
import { ProviderDeclaration } from './ProviderDeclaration';
import type * as S from '../api/schemas';
import { RunView } from '../features/RunView';
import { MemoryView } from '../features/Panels';
import { SourcesView } from '../features/SourcesView';
import { ModeBanner } from './ModeBanner';
import { LiveRegionProvider, useAnnounce } from '../a11y/AnnounceContext';
import { t } from '../i18n/es419';

type Profile = z.infer<typeof S.Profile>;
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

function Shell() {
  const hash = useHash();
  const announce = useAnnounce();
  const [profile, setProfile] = useState<Profile | null>(null);
  const [simulated, setSimulated] = useState(false);
  const [provider, setProvider] = useState('unknown');
  const [session, setSession] = useState<SessionState>('loading');
  const [debugApi, setDebugApi] = useState<DebugApi | null>(null);
  const [dataProvider, setDataProvider] = useState<string>('http');
  useEffect(() => {
    // public/config.json is the only source of the client provider (read at runtime, not baked into the bundle).
    let live = true;
    const off = onSessionExpired(() => setSession('expired'));
    void loadConfig().then(async (c) => {
      if (!live) return;
      setProvider(c.provider);
      const port = await bootDebugApi({ provider: c.dataProvider, baseUrl: c.apiBase, onSessionExpired: reportSessionExpired });
      if (!live) return;
      setDebugApi(port);
      setDataProvider(c.dataProvider);
      if (c.dataProvider === 'http') {
        // Legacy client path (other screens still use it): unchanged behaviour.
        api.session()
          .then((s) => { setCsrf(s.csrf_token); setSimulated(s.auth.simulated); setSession((cur) => (cur === 'expired' ? cur : 'ok')); })
          .catch((e: unknown) => { setCsrf(''); setSession(e instanceof ApiError && e.status === 401 ? 'expired' : 'unavailable'); });
        api.profile().then(setProfile).catch(() => setProfile(null));
      } else {
        // fixture / stand-in providers are in-process: session and mode come through the port.
        port.session().then((s) => { setSimulated(s.auth.simulated); setSession('ok'); }).catch(() => setSession('unavailable'));
        port.mode().then((m) => setProfile({ target: m.target, runtime_profile: m.runtime_profile, doubles: m.doubles, pin: null, ...(m.provider === 'stand-in' ? { mode: 'stand_in' } : {}) })).catch(() => setProfile(null));
      }
    });
    return () => { live = false; off(); };
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
      {debugApi && <DebugApiProvider api={debugApi}><ProviderDeclaration /></DebugApiProvider>}
      {(session === 'expired' || session === 'unavailable') && (
        <div className="banner bad" data-testid="session-banner" data-state={session}>
          {t(session === 'expired' ? 'session.expired' : 'session.unavailable')}
        </div>
      )}
      <nav aria-label={t('nav.label')}><a href="#/">{t('nav.runs')}</a> · <a href="#/memory">{t('nav.memory')}</a> · <a href="#/sources">{t('sources.nav')}</a></nav>
      {dataProvider !== 'http' && (run || path === '/sources' || path === '/memory') && (
        <div className="banner bad" role="note" data-testid="provider-partial">{t('decl.partial', { provider: dataProvider })}</div>
      )}
      <main>
        {run?.[1]
          ? <RunView
              runId={run[1]} nodeId={node}
              onNode={(id) => { window.location.hash = id ? `#/run/${run[1]}?node=${id}` : `#/run/${run[1]}`; }}
            />
          : path === '/sources' ? <SourcesView /> : path === '/memory' ? <MemoryView /> : <><h1>{t('app.title')}</h1>{debugApi ? <DebugApiProvider api={debugApi}><RunList /></DebugApiProvider> : <p>{t('runs.loading')}</p>}</>}
      </main>
    </>
  );
}

export function App() {
  return <LiveRegionProvider><Shell /></LiveRegionProvider>;
}
