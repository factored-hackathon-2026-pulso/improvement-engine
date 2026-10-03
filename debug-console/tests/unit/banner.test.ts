import { describe, expect, it } from 'vitest';
import { bannerLevel } from '../../src/app/banner';

const p = (doubles: string[]) => ({ target: 'real_local', runtime_profile: 'agent_core_real', doubles, pin: null });
describe('ModeBanner four combinations (plan 17.3.7)', () => {
  it('client real + no server profile -> unverified (red)', () => expect(bannerLevel('real', null)).toBe('unverified'));
  it('client real + doubles declared -> unverified (red)', () => expect(bannerLevel('real', p(['api_fixture']))).toBe('unverified'));
  it('client real + clean profile -> verified', () => expect(bannerLevel('real', p([]))).toBe('verified'));
  it('client fixture + doubles -> neutral fixture banner, never green-real', () => expect(bannerLevel('fixture', p(['api_fixture']))).toBe('fixture'));
  it('client fixture + no profile -> unverified (a missing profile is never silent)', () => expect(bannerLevel('fixture', null)).toBe('unverified'));
});
