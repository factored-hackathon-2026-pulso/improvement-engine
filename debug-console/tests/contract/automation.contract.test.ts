import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import * as S from '../../src/features/automation/schemas';
import * as DS from '../../src/api/schemas';
import { getJson, startTarget, stopTarget } from './helpers';

// Same technique as the debug-api contract suite: the fixture server and the Rust debug-api must satisfy the same zod
// schemas. CONTRACT_TARGET=real CONTRACT_BASE_URL=http://127.0.0.1:4021 (debug-api started with --automation-sim and
// --automation-proposals; CONTRACT_ADMIN_TOKEN = its DEBUG_API_ADMIN_TOKEN) runs it against the Rust server.
const A = '/internal/v1/automation';
const ADMIN = process.env.CONTRACT_ADMIN_TOKEN ?? 'adm';
beforeAll(startTarget, 30000);
afterAll(stopTarget);

const csrf = async () => ((await getJson('/api/v1/auth/session')).body as { csrf_token: string }).csrf_token;
const post = async (path: string, body: unknown, withCsrf = true) =>
  getJson(`${A}${path}`, { method: 'POST', body: JSON.stringify(body), headers: { 'content-type': 'application/json', ...(withCsrf ? { 'x-csrf-token': await csrf() } : {}) } });

describe('read model', () => {
  it('case-types validates, is labelled simulated and proposes an agent for Cobro indebido', async () => {
    const r = await getJson(`${A}/case-types`);
    expect(r.status).toBe(200);
    const p = S.CaseTypeList.safeParse(r.body);
    expect(p.success, p.success ? '' : JSON.stringify(p.error.issues)).toBe(true);
    const b = S.CaseTypeList.parse(r.body);
    expect(b.case_types.map((c) => c.type_id)).toContain('cobro_indebido');
    expect(b.doubles.length).toBeGreaterThan(0);
    expect(b.banner?.type_id).toBe('cobro_indebido');
    for (const c of b.case_types) if (c.measure.kind !== 'none') expect(c.measure.source, c.type_id).toBeTruthy();
  });
  it('every case type has a valid detail with the three thresholds and a draft window verdict', async () => {
    const b = S.CaseTypeList.parse((await getJson(`${A}/case-types`)).body);
    for (const c of b.case_types) {
      const r = await getJson(`${A}/case-types/${c.type_id}`);
      expect(r.status, c.type_id).toBe(200);
      const d = S.CaseTypeDetail.safeParse(r.body);
      expect(d.success, `${c.type_id}: ${d.success ? '' : JSON.stringify(d.error.issues)}`).toBe(true);
      expect(d.success && d.data.thresholds_today.length).toBe(3);
    }
  });
  it('unknown case type is a 404 Problem', async () => {
    const r = await getJson(`${A}/case-types/zzz`);
    expect(r.status).toBe(404);
    expect(DS.Problem.safeParse(r.body).success).toBe(true);
  });
});

describe('thresholds config (admin)', () => {
  const put = (b: unknown, token?: string) => getJson(`${A}/config`, { method: 'PUT', body: JSON.stringify(b), headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) } });
  it('is not open: no token is 401 (or 404 when the admin surface is off)', async () => {
    expect([401, 404]).toContain((await put({ repeat_q_min_cases: 25 })).status);
  });
  it('rejects invalid thresholds with a 422 Problem', async () => {
    const r = await put({ k_min: 3 }, ADMIN);
    if (r.status === 404) return;
    expect(r.status).toBe(422);
    expect(DS.Problem.safeParse(r.body).success).toBe(true);
  });
});

describe('simulated approve and publish-to-staging', () => {
  const hashOf = async () => S.CaseTypeDetail.parse((await getJson(`${A}/case-types/cobro_indebido`)).body).proposal!.candidate_hash;
  it('needs CSRF', async () => {
    expect((await post('/case-types/cobro_indebido/proposal/approve', { candidate_hash: 'x' }, false)).status).toBe(403);
  });
  it('publishing before approval is a 409 and a wrong hash is a 409', async () => {
    expect((await post('/case-types/cobro_indebido/proposal/publish-staging', {})).status).toBe(409);
    expect((await post('/case-types/cobro_indebido/proposal/approve', { candidate_hash: 'sha256:bad' })).status).toBe(409);
  });
  it('approve then publish, each answering simulated:true', async () => {
    const a = await post('/case-types/cobro_indebido/proposal/approve', { candidate_hash: await hashOf(), step_up: 'simulated' });
    expect(a.status).toBe(200);
    expect(S.ActionResult.parse(a.body)).toMatchObject({ state: 'approved_simulated', simulated: true });
    const p = await post('/case-types/cobro_indebido/proposal/publish-staging', {});
    expect(S.ActionResult.parse(p.body)).toMatchObject({ state: 'staged_simulated', simulated: true });
    expect(S.CaseTypeDetail.parse((await getJson(`${A}/case-types/cobro_indebido`)).body).proposal?.state).toBe('staged_simulated');
  });
});
