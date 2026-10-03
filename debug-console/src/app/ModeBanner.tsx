import type { z } from 'zod';
import type * as S from '../api/schemas';
import { bannerLevel } from './banner';
import { t } from '../i18n/es419';
import { doubleLabel } from '../features/DemoPanels';

type Profile = z.infer<typeof S.Profile>;

/** Always-visible mode banner. Not a live region: changes are announced through the single announcer. */
export function ModeBanner({ profile, simulated, provider }: { profile: Profile | null; simulated: boolean; provider: string }) {
  const level = bannerLevel(provider, profile);
  if (!profile) return <div className="banner bad" data-testid="mode-banner" data-level={level}>{t('banner.unverified')}</div>;
  if (level === 'stand_in') {
    return (
      <div className="banner stand_in" data-testid="mode-banner" data-level={level}>
        {t('banner.standIn', { profile: profile.runtime_profile, doubles: profile.doubles.map(doubleLabel).join(', ') || t('banner.none') })}
        {profile.decision_hook === 'pending' && t('banner.hookPending')}
        {simulated && t('banner.simulated')}
        {profile.doubles_detail && profile.doubles_detail.length > 0 && (
          <details data-testid="doubles-detail">
            <summary>{t('banner.doublesDetail')}</summary>
            <ul>
              {profile.doubles_detail.map((d) => (
                <li key={d.id}><strong>{doubleLabel(d.id)}</strong>: {d.what}{d.until ? ` (${t('banner.until', { value: d.until })})` : ''}</li>
              ))}
            </ul>
          </details>
        )}
      </div>
    );
  }
  return (
    <div className={level === 'unverified' ? 'banner bad' : 'banner'} data-testid="mode-banner" data-level={level}>
      {t('banner.mode', { target: profile.target, profile: profile.runtime_profile, doubles: profile.doubles.join(', ') || t('banner.none') })}
      {level === 'unverified' && ` · ${t('banner.unverified')}`}
      {simulated && t('banner.simulated')}
    </div>
  );
}
