import { createContext, useContext, useMemo, useState, type ReactNode } from 'react';
import { createAnnouncer, type Announcer } from './announcer';

const noop: Announcer = { announce: () => undefined };
const Ctx = createContext<Announcer>(noop);
/** Announce through the single app-wide live region. Outside a provider this is a no-op. */
export const useAnnounce = (): Announcer['announce'] => useContext(Ctx).announce;

/** Owns the ONLY aria-live region of the app (polite, rate limited). Everything else is silent text. */
export function LiveRegionProvider({ children, minIntervalMs = 2000 }: { children: ReactNode; minIntervalMs?: number }) {
  const [text, setText] = useState('');
  const announcer = useMemo(() => createAnnouncer(setText, minIntervalMs), [minIntervalMs]);
  return (
    <Ctx.Provider value={announcer}>
      {children}
      <div role="status" aria-live="polite" aria-atomic="true" className="sr-only" data-testid="live-region">{text}</div>
    </Ctx.Provider>
  );
}
