import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const read = (p: string) => readFileSync(join(process.cwd(), p), 'utf8');
const inc = read('nginx/security-headers.inc');
const csp = /add_header Content-Security-Policy "([^"]+)"/.exec(inc)?.[1] ?? '';
const directive = (name: string) => csp.split(';').map((d) => d.trim()).find((d) => d.startsWith(`${name} `)) ?? '';

describe('nginx CSP and hardening headers', () => {
  it('has no unsafe-inline / unsafe-eval and denies by default', () => {
    expect(csp).not.toBe('');
    expect(csp).not.toMatch(/unsafe-inline|unsafe-eval/);
    expect(directive('default-src')).toBe("default-src 'none'");
    expect(directive('script-src')).toBe("script-src 'self'");
    expect(directive('connect-src')).toBe("connect-src 'self'");
    expect(directive('frame-ancestors')).toBe("frame-ancestors 'none'");
    expect(directive('object-src')).toBe("object-src 'none'");
    expect(directive('base-uri')).toBe("base-uri 'none'");
    expect(directive('form-action')).toBe("form-action 'self'");
  });
  it('sets the other security headers', () => {
    expect(inc).toContain('X-Content-Type-Options "nosniff"');
    expect(inc).toContain('Referrer-Policy "no-referrer"');
    expect(inc).toContain('Cross-Origin-Opener-Policy "same-origin"');
  });
  it('index.html has no inline script, so the CSP holds', () => {
    const html = read('index.html');
    expect(html).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>/i);
    expect(html).not.toMatch(/\son\w+=/i);
  });
  it('nginx template serves /healthz on 3000, proxies both prefixes unbuffered, includes the headers everywhere', () => {
    const t = read('nginx/default.conf.template');
    expect(t).toContain('listen 3000;');
    expect(t).toContain('location = /healthz');
    expect(t).toMatch(/location \/internal\/ \{[^}]*set \$upstream \$\{PULSO_CONTROL_API_URL\};[^}]*proxy_pass \$upstream[^}]*proxy_buffering off/s);
    expect(t).toMatch(/location \/api\/ \{[^}]*set \$upstream \$\{PULSO_CONTROL_API_URL\};[^}]*proxy_pass \$upstream/s);
    expect(t).toContain('resolver ${NGINX_LOCAL_RESOLVERS}'); // lazy DNS: the console starts even if pulso-api is not up yet
    const blocks = t.split(/\n  location /).slice(1);
    for (const b of blocks) expect(b, b.split('\n')[0]).toContain('security-headers.inc');
  });
  it('Dockerfile uses nginx-unprivileged, copies the lockfile and runs npm ci', () => {
    const d = read('Dockerfile');
    expect(d).toMatch(/nginx-unprivileged/);
    expect(d).toContain('COPY package.json package-lock.json');
    expect(d).toContain('npm ci');
    expect(d).toContain('EXPOSE 3000');
    expect(d).not.toMatch(/^USER root/m);
  });
});
