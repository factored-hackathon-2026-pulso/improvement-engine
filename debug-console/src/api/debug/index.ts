import { createHttpProvider, type FetchLike } from './http';
import type { Backend } from './backend';
import type { DebugApi, ProviderKind } from './port';

export * from './port';
export * from './commands';
export { DebugApiError } from './dto';

export interface DebugApiConfig {
  provider: ProviderKind;
  /** http: origin of the control-api (empty = same-origin). */
  baseUrl?: string;
  /** fixture / stand-in: the in-process backend serving the spec 25 routes. */
  backend?: Backend;
  fetch?: FetchLike;
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
  onSessionExpired?: () => void;
}

/** The only place that knows the three providers. Switching demo -> real is `provider: 'http'` plus `baseUrl`, not a rewrite. */
export function createDebugApi(cfg: DebugApiConfig): DebugApi {
  const common = { kind: cfg.provider, ...(cfg.sleep ? { sleep: cfg.sleep } : {}), ...(cfg.onSessionExpired ? { onSessionExpired: cfg.onSessionExpired } : {}) };
  if (cfg.provider === 'http') return createHttpProvider({ ...common, baseUrl: cfg.baseUrl ?? '', ...(cfg.fetch ? { fetch: cfg.fetch } : {}) });
  if (!cfg.backend) throw new Error(`provider ${cfg.provider} needs an in-process backend`);
  return createHttpProvider({ ...common, fetch: cfg.backend.fetch });
}

/** Boot-time selection from config. Fixture/stand-in worlds are loaded lazily so an http build never pulls the demo data in. */
export async function bootDebugApi(cfg: { provider: ProviderKind; baseUrl?: string; onSessionExpired?: () => void }): Promise<DebugApi> {
  if (cfg.provider === 'http') return createDebugApi(cfg);
  const [{ createBackend }, worlds] = await Promise.all([import('./backend'), import('./worlds')]);
  const backend = createBackend(cfg.provider === 'stand-in' ? worlds.standInWorld() : worlds.fixtureWorld());
  return createDebugApi({ ...cfg, backend });
}
