import { useEffect, useState } from 'react';
import { useDebugApi } from '../api/debug/context';
import type { RunList as RunListDto } from '../api/debug/port';
import { t } from '../i18n/es419';

/** Run list screen, migrated onto the DebugApi port (GET /internal/v1/debug/runs). */
export function RunList() {
  const api = useDebugApi();
  const [runs, setRuns] = useState<RunListDto | null>(null);
  const [err, setErr] = useState(false);
  useEffect(() => {
    let live = true;
    api.runs().then((r) => { if (live) setRuns(r); }).catch(() => { if (live) setErr(true); });
    return () => { live = false; };
  }, [api]);
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
