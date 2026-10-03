import type { z } from 'zod';
import type * as S from '../api/schemas';
import { bannerLevel } from './banner';
import { t } from '../i18n/es419';

type Profile = z.infer<typeof S.Profile>;

/** Always-visible mode banner. Not a live region: changes are announced through the single announcer. */
export function ModeBanner({ profile, simulated, provider }: { profile: Profile | null; simulated: boolean; provider: string }) {
  const level = bannerLevel(provider, profile);
  if (!profile) return <div className="banner bad" data-testid="mode-banner" data-level={level}>{t('banner.unverified')}</div>;
  return (
    <div className={level === 'unverified' ? 'banner bad' : 'banner'} data-testid="mode-banner" data-level={level}>
      {t('banner.mode', { target: profile.target, profile: profile.runtime_profile, doubles: profile.doubles.join(', ') || t('banner.none') })}
      {level === 'unverified' && ` · ${t('banner.unverified')}`}
      {simulated && t('banner.simulated')}
    </div>
  );
}
