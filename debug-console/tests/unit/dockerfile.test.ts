import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const df = readFileSync(join(process.cwd(), 'Dockerfile'), 'utf8');
const arg = (name: string) => new RegExp(`^ARG ${name}=(.+)$`, 'm').exec(df)?.[1] ?? '';

describe('Dockerfile reproducibility', () => {
  it.each(['NODE_IMAGE', 'NGINX_IMAGE'])('%s is pinned by sha256 digest', (name) => {
    expect(arg(name)).toMatch(/@sha256:[0-9a-f]{64}$/);
  });
  it('keeps the tag next to the digest for readability', () => {
    expect(arg('NODE_IMAGE')).toMatch(/^node:22\.16-alpine@sha256:/);
    expect(arg('NGINX_IMAGE')).toMatch(/^nginxinc\/nginx-unprivileged:1\.27-alpine@sha256:/);
  });
  it('documents that podman must build with --format docker or HEALTHCHECK is dropped', () => {
    expect(df).toMatch(/--format docker/);
    expect(df).toMatch(/^HEALTHCHECK /m);
  });
});
