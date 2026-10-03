import { expect, it } from 'vitest';
import { SOURCES_ES, SOURCES_EN } from '../../src/i18n/sources';

it('English dictionary has exactly the Spanish keys, none empty', () => {
  expect(Object.keys(SOURCES_EN).sort()).toEqual(Object.keys(SOURCES_ES).sort());
  for (const v of Object.values(SOURCES_EN)) expect(v.length).toBeGreaterThan(0);
});
