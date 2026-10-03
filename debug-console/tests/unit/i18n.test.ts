import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { ES_419, t } from '../../src/i18n/es419';

const SRC = join(process.cwd(), 'src');
const walk = (d: string): string[] => readdirSync(d).flatMap((f) => {
  const p = join(d, f);
  return statSync(p).isDirectory() ? walk(p) : p.endsWith('.tsx') ? [p] : [];
});
const files = walk(SRC);

describe('es-419 UI strings', () => {
  it('declares es-419 on the document', () => {
    expect(readFileSync(join(process.cwd(), 'index.html'), 'utf8')).toContain('lang="es-419"');
  });
  it('interpolates params and leaves unknown placeholders visible', () => {
    expect(t('run.title', { id: 'r1' })).toBe('Ejecución r1');
    expect(t('run.title')).toBe('Ejecución {id}');
  });
  it('has no English stop words in any string', () => {
    for (const [k, v] of Object.entries(ES_419)) expect(v, k).not.toMatch(/\b(the|and|loading|approve the|failed to|error)\b/i);
  });
  it('every t() key used in components exists, and components keep no hard-coded UI text', () => {
    for (const f of files) {
      const code = readFileSync(f, 'utf8');
      for (const m of code.matchAll(/\bt\('([\w.]+)'/g)) expect(Object.keys(ES_419), `${f}: ${m[1]}`).toContain(m[1]);
      // JSX text nodes with letters outside {expressions}: must come from t()
      const jsxText = [...code.matchAll(/>([^<>{}\n]*[A-Za-zÁÉÍÓÚáéíóúñ]{3,}[^<>{}\n]*)</g)].map((m) => m[1]!.trim()).filter((x) => x && !/[()=;&|]/.test(x) && x !== 'Promise');
      expect(jsxText, f).toEqual([]);
    }
  });
});
