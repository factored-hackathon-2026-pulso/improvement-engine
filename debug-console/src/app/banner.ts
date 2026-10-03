export type BannerLevel = 'verified' | 'unverified' | 'fixture' | 'stand_in';
interface ProfileLike { doubles: string[]; mode?: string | undefined }
/** Client provider vs server-declared profile. "real" with no profile or with doubles is never trusted. */
export function bannerLevel(provider: string, profile: ProfileLike | null): BannerLevel {
  if (!profile) return 'unverified';
  if (provider === 'real') return profile.doubles.length > 0 ? 'unverified' : 'verified';
  return profile.mode === 'stand_in' ? 'stand_in' : 'fixture';
}
