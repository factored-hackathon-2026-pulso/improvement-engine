// One live region for the whole app, rate limited: bursts coalesce into a single update and
// only nodes that changed are ever announced (never the whole graph again).
export interface Announcer { announce: (message: string) => void }

export function createAnnouncer(flush: (text: string) => void, minIntervalMs = 2000): Announcer {
  let lastFlush = -Infinity;
  let pending: string[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;

  const emit = () => {
    timer = null;
    if (pending.length === 0) return;
    lastFlush = Date.now();
    flush(pending.join('. '));
    pending = [];
  };
  return {
    announce(message) {
      if (!pending.includes(message)) pending.push(message);
      if (pending.length > 6) pending = pending.slice(-6);
      if (timer !== null) return;
      const wait = lastFlush + minIntervalMs - Date.now();
      if (wait <= 0) emit();
      else timer = setTimeout(emit, wait);
    },
  };
}

interface NodeLike { node_id: string; label: string; status: string }
export function diffNodes(prev: NodeLike[] | null, next: NodeLike[]): { label: string; status: string }[] {
  if (!prev) return [];
  const before = new Map(prev.map((n) => [n.node_id, n.status]));
  return next.filter((n) => before.get(n.node_id) !== n.status).map((n) => ({ label: n.label, status: n.status }));
}
