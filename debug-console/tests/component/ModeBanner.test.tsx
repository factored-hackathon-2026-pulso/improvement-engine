import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { ModeBanner } from '../../src/app/ModeBanner';

const profile = (doubles: string[]) => ({ target: 'real_local', runtime_profile: 'agent_core_real', doubles, pin: null });
const banner = () => screen.getByTestId('mode-banner');
afterEach(cleanup);

describe('ModeBanner render (four combinations, plan 17.3.7)', () => {
  it('client real + no server profile: red, "Perfil sin verificar", never green', () => {
    render(<ModeBanner profile={null} simulated={false} provider="real" />);
    expect(banner().getAttribute('data-level')).toBe('unverified');
    expect(banner().className).toContain('bad');
    expect(banner().textContent).toContain('Perfil sin verificar');
  });
  it('client real + doubles declared: red with the doubles listed', () => {
    render(<ModeBanner profile={profile(['api_fixture'])} simulated={false} provider="real" />);
    expect(banner().getAttribute('data-level')).toBe('unverified');
    expect(banner().className).toContain('bad');
    expect(banner().textContent).toContain('api_fixture');
    expect(banner().textContent).toContain('Perfil sin verificar');
  });
  it('client real + clean profile: verified, neutral class, "ninguno" for doubles', () => {
    render(<ModeBanner profile={profile([])} simulated={false} provider="real" />);
    expect(banner().getAttribute('data-level')).toBe('verified');
    expect(banner().className).not.toContain('bad');
    expect(banner().textContent).toContain('ninguno');
    expect(banner().textContent).not.toContain('Perfil sin verificar');
  });
  it('client fixture + doubles: fixture level, never verified, simulated identity labelled', () => {
    render(<ModeBanner profile={profile(['api_fixture'])} simulated provider="fixture" />);
    expect(banner().getAttribute('data-level')).toBe('fixture');
    expect(banner().textContent).toContain('auth.simulated=true');
    expect(banner().textContent).toContain('real_local');
  });
  it('client fixture + no profile: unverified, a missing profile is never silent', () => {
    render(<ModeBanner profile={null} simulated={false} provider="fixture" />);
    expect(banner().getAttribute('data-level')).toBe('unverified');
  });
  it('is not a live region (announcements go through the single announcer)', () => {
    render(<ModeBanner profile={null} simulated={false} provider="real" />);
    expect(banner().getAttribute('role')).toBeNull();
    expect(banner().getAttribute('aria-live')).toBeNull();
  });
});
