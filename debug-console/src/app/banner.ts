export type BannerLevel = 'verified' | 'unverified' | 'fixture';
interface ProfileLike { doubles: string[] }
/** Client provider vs server-declared profile. "real" with no profile or with doubles is never trusted. */
export function bannerLevel(provider: string, profile: ProfileLike | null): BannerLevel {
  if (!profile) return 'unverified';
  if (provider === 'real') return profile.doubles.length > 0 ? 'unverified' : 'verified';
  return 'fixture';
}
