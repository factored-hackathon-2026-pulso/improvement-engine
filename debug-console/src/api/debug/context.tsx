import { createContext, useContext, type ReactNode } from 'react';
import type { DebugApi } from './port';

const Ctx = createContext<DebugApi | null>(null);
/** Screens get their data only from the port provided here; none of them imports fetch or a concrete provider. */
export function DebugApiProvider({ api, children }: { api: DebugApi; children: ReactNode }) {
  return <Ctx.Provider value={api}>{children}</Ctx.Provider>;
}
export function useDebugApi(): DebugApi {
  const v = useContext(Ctx);
  if (!v) throw new Error('DebugApiProvider missing');
  return v;
}
