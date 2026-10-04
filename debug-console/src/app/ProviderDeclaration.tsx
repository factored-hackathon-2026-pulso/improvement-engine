import { useEffect, useState } from 'react';
import { useDebugApi } from '../api/debug/context';
import type { ModeDeclaration } from '../api/debug/port';
import { t } from '../i18n/es419';

/** Declares the active data provider (provider/target/runtime_profile/doubles[]). A failed read is "unverified", never assumed real. */
export function ProviderDeclaration() {
  const api = useDebugApi();
  const [mode, setMode] = useState<ModeDeclaration | null | 'failed'>(null);
  useEffect(() => {
    let live = true;
    api.mode().then((m) => { if (live) setMode(m); }).catch(() => { if (live) setMode('failed'); });
    return () => { live = false; };
  }, [api]);
  if (mode === null) return null;
  if (mode === 'failed') return <div className="banner bad" role="note" aria-label={t('decl.label')} data-testid="provider-declaration" data-provider="unverified">{t('decl.unverified')}</div>;
  return (
    <div className={mode.provider === 'http' ? 'banner' : 'banner stand_in'} role="note" aria-label={t('decl.label')} data-testid="provider-declaration" data-provider={mode.provider}>
      {t('decl.text', { provider: mode.provider, target: mode.target, profile: mode.runtime_profile, doubles: mode.doubles.join(', ') || t('banner.none') })}
    </div>
  );
}
