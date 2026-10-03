import { useEffect, useState } from 'react';
import type { z } from 'zod';
import { api, setCsrf } from '../api/client';
import type * as S from '../api/schemas';
import { RunView } from '../features/RunView';
import { MemoryView } from '../features/Panels';

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
  if (!profile) return <div className="banner bad" role="status">Perfil sin verificar</div>;
  const bad = profile.doubles.length > 0 && profile.target === 'real';
  return (
    <div className={bad ? 'banner bad' : 'banner'} data-testid="mode-banner">
      Modo: {profile.target} · {profile.runtime_profile} · dobles: {profile.doubles.join(', ') || 'ninguno'}
      {simulated && ' · identidad de prueba (auth.simulated=true)'}
    </div>
  );
}

function RunList() {
  const [runs, setRuns] = useState<Runs | null>(null);
  const [err, setErr] = useState(false);
  useEffect(() => { api.runs().then(setRuns).catch(() => setErr(true)); }, []);
  if (err) return <p>unknown · no se pudo leer la lista</p>;
  if (!runs) return <p>Cargando…</p>;
  if (runs.items.length === 0) return <p>Sin runs: unknown · no implica ingesta completa</p>;
  return (
    <ul aria-label="Runs">
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
      <nav aria-label="Principal"><a href="#/">Runs</a> · <a href="#/memory">Memoria</a></nav>
      <main>
        {run?.[1]
          ? <RunView
              runId={run[1]} nodeId={node}
              onNode={(id) => { window.location.hash = id ? `#/run/${run[1]}?node=${id}` : `#/run/${run[1]}`; }}
            />
          : path === '/memory' ? <MemoryView /> : <><h1>Pulso debug console</h1><RunList /></>}
      </main>
    </>
  );
}
