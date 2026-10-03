import { spawn, type ChildProcess } from 'node:child_process';

export const TARGET = (process.env.CONTRACT_TARGET ?? 'fixture') as 'fixture' | 'real';
const PORT = process.env.CONTRACT_FIXTURE_PORT ?? '4011';
export const BASE = process.env.CONTRACT_BASE_URL ?? (TARGET === 'fixture' ? `http://127.0.0.1:${PORT}` : '');
export const DEBUG = '/internal/v1/debug';

let child: ChildProcess | null = null;
export async function startTarget(): Promise<void> {
  if (TARGET === 'real') {
    if (!BASE) throw new Error('CONTRACT_TARGET=real requires CONTRACT_BASE_URL');
    return;
  }
  child = spawn(process.execPath, ['fixture-server/server.mjs'], { env: { ...process.env, FIXTURE_PORT: PORT }, stdio: 'ignore' });
  for (let i = 0; i < 50; i += 1) {
    try { if ((await fetch(`${BASE}/healthz`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('fixture server did not start');
}
export const stopTarget = () => { child?.kill(); child = null; };
export const fixtureOnly = TARGET === 'fixture';
export async function getJson(path: string, init?: RequestInit): Promise<{ status: number; body: unknown; headers: Headers }> {
  const res = await fetch(`${BASE}${path}`, init);
  return { status: res.status, body: await res.json().catch(() => null), headers: res.headers };
}
