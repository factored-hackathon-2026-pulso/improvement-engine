import { describe, expect, it } from 'vitest';
import { scrub, MASK } from '../../src/security/clientScrubber';

const jws = 'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJlXzEyMzQ1';
describe('clientScrubber', () => {
  it('masks JWS-shaped strings', () => {
    const r = scrub({ note: `see ${jws} now` });
    expect(JSON.stringify(r.value)).not.toContain('eyJ');
    expect(r.hits).toEqual(['jws']);
  });
  it('masks Bearer tokens and DSN-shaped strings', () => {
    const r = scrub(['Authorization: Bearer abc.DEF-123_xyz==', 'postgres://user:pw@host:5432/db', 'https://k@o1.ingest.sentry.io/42']);
    expect(r.value).toEqual([`Authorization: ${MASK}`, MASK, MASK]);
    expect(r.hits.sort()).toEqual(['bearer', 'dsn', 'dsn']);
  });
  it('masks final_locked refs and canaries', () => {
    const r = scrub({ a: 'ref final_locked:holdout/abc', b: 'CANARY_SECRET_123e4567-e89b-12d3-a456-426614174000' });
    expect(JSON.stringify(r.value)).not.toMatch(/final_locked:holdout|CANARY_/);
    expect(r.hits.sort()).toEqual(['canary', 'final_locked']);
  });
  it('replaces objects whose visibility is final_locked', () => {
    const r = scrub({ items: [{ id: 1, visibility: 'final_locked', body: 'x' }, { id: 2, visibility: 'public', body: 'y' }] });
    expect(r.value).toEqual({ items: [{ redacted: true, reason: 'final_locked' }, { id: 2, visibility: 'public', body: 'y' }] });
  });
  it('is pure, deep, leaves clean input untouched and tolerates non-strings', () => {
    const input = { n: 1, b: true, z: null, s: 'plain', arr: [1, { k: 'v' }] };
    const r = scrub(input);
    expect(r.value).toEqual(input);
    expect(r.hits).toEqual([]);
  });
  it('masks keys that look like secrets', () => {
    const r = scrub({ password: 'hunter2', api_key: 'k', note: 'ok' });
    expect(r.value).toEqual({ password: MASK, api_key: MASK, note: 'ok' });
    expect(r.hits).toEqual(['secret_key', 'secret_key']);
  });
});
