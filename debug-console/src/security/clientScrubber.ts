// Defence in depth: redaction is the server's job; this masks anything that still looks sensitive.
export const MASK = '[redacted]';
export type HitKind = 'jws' | 'bearer' | 'dsn' | 'final_locked' | 'canary' | 'secret_key';
export interface ScrubResult { value: unknown; hits: HitKind[] }

const RULES: [HitKind, RegExp][] = [
  ['bearer', /\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/g],
  ['jws', /\beyJ[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}/g],
  ['dsn', /\b[a-z][a-z0-9+.-]*:\/\/[^\s/@:]+(?::[^\s/@]*)?@[^\s]+/gi],
  ['final_locked', /\bfinal_locked[:/][\w./-]+/g],
  ['canary', /CANARY_(?:FINAL|PII|SECRET)_[0-9a-f-]{8,}/gi],
];
const SECRET_KEY = /^(?:password|passwd|secret|token|api[_-]?key|authorization|client_secret|private_key)$/i;

function scrubString(s: string, hits: HitKind[]): string {
  let out = s;
  for (const [kind, re] of RULES) {
    out = out.replace(re, () => { hits.push(kind); return MASK; });
  }
  return out;
}

function walk(v: unknown, hits: HitKind[]): unknown {
  if (typeof v === 'string') return scrubString(v, hits);
  if (Array.isArray(v)) return v.map((x) => walk(x, hits));
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    if (o.visibility === 'final_locked') { hits.push('final_locked'); return { redacted: true, reason: 'final_locked' }; }
    const out: Record<string, unknown> = {};
    for (const [k, x] of Object.entries(o)) {
      if (SECRET_KEY.test(k) && typeof x === 'string') { hits.push('secret_key'); out[k] = MASK; } else out[k] = walk(x, hits);
    }
    return out;
  }
  return v;
}

export function scrub(input: unknown): ScrubResult {
  const hits: HitKind[] = [];
  const value = walk(input, hits);
  return { value, hits };
}

/** Scrub and, when anything was masked, report once to the console without echoing the value. */
export function scrubAndReport(input: unknown, where: string): unknown {
  const r = scrub(input);
  if (r.hits.length > 0) console.error('security.unredacted_payload', { where, kinds: [...new Set(r.hits)], count: r.hits.length });
  return r.value;
}
