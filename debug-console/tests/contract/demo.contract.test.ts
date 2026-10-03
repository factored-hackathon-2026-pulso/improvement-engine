import { spawn, type ChildProcess } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import * as S from '../../src/api/schemas';

// Demo world support in the fixture server: FIXTURE_WORLD_FILE at boot and POST /__fixture/load at runtime.
const PORT = '4012';
const BASE = `http://127.0.0.1:${PORT}`;
const DEBUG = '/internal/v1/debug';
const WORLD_FILE = 'fixtures/demo-world.json';
let child: ChildProcess | null = null;
const get = async (p: string) => { const r = await fetch(BASE + p); return { status: r.status, body: (await r.json().catch(() => null)) as Record<string, any> }; };
const post = async (p: string, data: unknown) => {
  const r = await fetch(BASE + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(data) });
  return { status: r.status, body: (await r.json().catch(() => null)) as Record<string, any> };
};
async function boot(env: Record<string, string>) {
  child = spawn(process.execPath, ['fixture-server/server.mjs'], { env: { ...process.env, FIXTURE_PORT: PORT, ...env }, stdio: 'ignore' });
  for (let i = 0; i < 50; i += 1) {
    try { if ((await fetch(`${BASE}/healthz`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('fixture server did not start');
}
afterAll(() => { child?.kill(); });

describe('FIXTURE_WORLD_FILE', () => {
  beforeAll(() => boot({ FIXTURE_WORLD_FILE: WORLD_FILE }), 30000);

  it('serves the loaded world without wrapper hacks', async () => {
    const runs = S.RunList.parse((await get(`${DEBUG}/runs`)).body);
    expect(runs.items.map((r) => r.run_id)).toEqual(['run-demo', 'run-demo-refuted', 'run-demo-successor']);
  });
  it('profile is driven by world.demo: mode, runtime profile, doubles and their honest descriptions', async () => {
    const p = S.Profile.parse((await get(`${DEBUG}/profile`)).body);
    expect(p.mode).toBe('stand_in');
    expect(p.runtime_profile).toBe('real_local_core_standin');
    expect(p.doubles).toEqual(expect.arrayContaining(['stand_in_engine', 'scripted_llm', 'fixture_api']));
    expect(p.doubles_detail?.find((d) => d.id === 'stand_in_engine')?.until).toBe('Rust engine HTTP API exists');
    expect(p.decision_hook).toBe('pending');
  });
  it('gates are per run: the run that evaluated carries the attempt history, a refuted run is not evaluable', async () => {
    const g = S.Gates.parse((await get(`${DEBUG}/runs/run-demo/gates`)).body);
    expect(g.proposal_id).toBe('prop-1');
    expect(g.attempts?.map((a) => [a.attempt, a.improvement.status])).toEqual([[1, 'fail'], [2, 'pass']]);
    expect(g.native.report_ref?.digest).toMatch(/^[0-9a-f]{64}$/);
    const r = S.Gates.parse((await get(`${DEBUG}/runs/run-demo-refuted/gates`)).body);
    expect(r.native.status).toBe('not_evaluable');
    expect(r.proposal_id ?? null).toBeNull();
    expect(r.attempts ?? []).toEqual([]);
  });
  it('alternatives (do nothing vs proposed change) come from world.demo and only for the evaluated run', async () => {
    const a = S.Alternatives.parse((await get(`${DEBUG}/runs/run-demo/alternatives`)).body);
    expect(a.items.map((i) => i.kind)).toEqual(['do_nothing', 'proposed_change']);
    expect(a.items[0]!.expected_abandoned!).toBeGreaterThan(a.items[1]!.expected_abandoned!);
    expect(S.Alternatives.parse((await get(`${DEBUG}/runs/run-demo-refuted/alternatives`)).body).items).toEqual([]);
  });
  it('diff is per proposal id, never a hardcoded prop-1', async () => {
    S.Diff.parse((await get(`${DEBUG}/proposals/prop-1/diff`)).body);
    expect((await get(`${DEBUG}/proposals/nope/diff`)).status).toBe(404);
  });
  it('investigation and graph validate for every demo run', async () => {
    for (const id of ['run-demo', 'run-demo-refuted', 'run-demo-successor']) {
      S.Investigation.parse((await get(`${DEBUG}/runs/${id}/investigation`)).body);
      S.Graph.parse((await get(`${DEBUG}/runs/${id}/graph`)).body);
    }
  });
});

describe('POST /__fixture/load', () => {
  beforeAll(async () => { child?.kill(); await new Promise((r) => setTimeout(r, 300)); await boot({}); }, 30000);

  it('loads a world at runtime, and a later reset returns to the scenarios', async () => {
    const world = JSON.parse(readFileSync(WORLD_FILE, 'utf8'));
    expect((await post('/__fixture/load', world)).status).toBe(200);
    expect(S.RunList.parse((await get(`${DEBUG}/runs`)).body).items).toHaveLength(3);
    expect(S.Profile.parse((await get(`${DEBUG}/profile`)).body).mode).toBe('stand_in');
    await post('/__fixture/reset', { scenario: 'default' });
    expect(S.Profile.parse((await get(`${DEBUG}/profile`)).body).mode ?? null).toBeNull();
  });
  it('rejects a body that is not a world and keeps the current one', async () => {
    const bad = await post('/__fixture/load', { runs: 'nope' });
    expect(bad.status).toBe(400);
    expect(bad.body.code).toBe('invalid_world');
    expect(S.RunList.parse((await get(`${DEBUG}/runs`)).body).items.length).toBeGreaterThan(0);
  });
  it('the default world keeps serving gates for every run and diff prop-1 (back compat)', async () => {
    const g = S.Gates.parse((await get(`${DEBUG}/runs/run-refuted/gates`)).body);
    expect(g.native.status).toBe('pass');
    expect(g.proposal_id).toBe('prop-1');
    expect(S.Alternatives.parse((await get(`${DEBUG}/runs/run-active/alternatives`)).body).items).toEqual([]);
  });
});
