// Test harness: the same DebugApi contract suite runs against three providers.
// fixture and stand-in talk to the in-process backend; http talks to a real local node http server wrapping the same backend.
import http from 'node:http';
import type { AddressInfo, Socket } from 'node:net';
import { createBackend, type Backend } from '../../../src/api/debug/backend';
import { fixtureWorld, standInWorld } from '../../../src/api/debug/worlds';
import { createDebugApi } from '../../../src/api/debug';
import type { DebugApi, ProviderKind } from '../../../src/api/debug/port';

export interface Harness { api: DebugApi; backend: Backend; close: () => Promise<void> }
export const noSleep = () => Promise.resolve();

function serve(backend: Backend): Promise<{ base: string; close: () => Promise<void> }> {
  const sockets = new Set<Socket>();
  const server = http.createServer((req, res) => {
    const ctl = new AbortController();
    res.on('close', () => ctl.abort());
    const headers = new Headers();
    for (const [k, v] of Object.entries(req.headers)) if (typeof v === 'string') headers.set(k, v);
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => {
      const body = chunks.length ? Buffer.concat(chunks) : undefined;
      const hasBody = body !== undefined && req.method !== 'GET';
      const request = new Request(`http://localhost${req.url ?? '/'}`, {
        method: req.method ?? 'GET', headers, signal: ctl.signal, ...(hasBody ? { body } : {}),
      });
      void backend.handle(request).then(async (r) => {
        res.writeHead(r.status, Object.fromEntries(r.headers.entries()));
        if (!r.body) { res.end(); return; }
        const reader = r.body.getReader();
        try { for (;;) { const { value, done } = await reader.read(); if (done) break; res.write(value); } } catch { /* client gone */ }
        res.end();
      });
    });
  });
  server.on('connection', (s) => { sockets.add(s); s.on('close', () => sockets.delete(s)); });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok({
    base: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
    close: () => new Promise<void>((done) => { backend.closeStreams(); sockets.forEach((s) => s.destroy()); server.close(() => done()); }),
  })));
}

export async function makeHarness(kind: ProviderKind): Promise<Harness> {
  const backend = createBackend(kind === 'stand-in' ? standInWorld() : fixtureWorld());
  if (kind === 'http') {
    const s = await serve(backend);
    return { api: createDebugApi({ provider: 'http', baseUrl: s.base, sleep: noSleep }), backend, close: s.close };
  }
  return { api: createDebugApi({ provider: kind, backend, sleep: noSleep }), backend, close: async () => backend.closeStreams() };
}
