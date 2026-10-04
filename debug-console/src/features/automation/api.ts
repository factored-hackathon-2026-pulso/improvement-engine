import type { z } from 'zod';
import { api as legacy } from '../../api/client';
import * as S from './schemas';

export const AUTOMATION = '/internal/v1/automation';

/** The seam the view depends on, so it can run on the Rust read model, the fixture server or a test double. */
export interface AutomationApi {
  caseTypes(): Promise<S.CaseTypeList>;
  caseType(id: string): Promise<S.CaseTypeDetail>;
  approve(id: string, candidateHash: string): Promise<S.ActionResult>;
  publishStaging(id: string): Promise<S.ActionResult>;
}

async function call<T>(schema: z.ZodType<T, z.ZodTypeDef, unknown>, path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${AUTOMATION}${path}`, { ...init, credentials: 'same-origin', cache: 'no-store' });
  if (!res.ok) throw new Error(`automation_${res.status}`);
  return schema.parse(await res.json());
}

async function post<T>(schema: z.ZodType<T, z.ZodTypeDef, unknown>, path: string, body: unknown): Promise<T> {
  const csrf = (await legacy.session()).csrf_token;
  return call(schema, path, { method: 'POST', headers: { 'content-type': 'application/json', 'x-csrf-token': csrf }, body: JSON.stringify(body) });
}

export const httpAutomationApi: AutomationApi = {
  caseTypes: () => call(S.CaseTypeList, '/case-types'),
  caseType: (id) => call(S.CaseTypeDetail, `/case-types/${encodeURIComponent(id)}`),
  approve: (id, candidateHash) => post(S.ActionResult, `/case-types/${encodeURIComponent(id)}/proposal/approve`, { candidate_hash: candidateHash, step_up: 'simulated' }),
  publishStaging: (id) => post(S.ActionResult, `/case-types/${encodeURIComponent(id)}/proposal/publish-staging`, {}),
};
