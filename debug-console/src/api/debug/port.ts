import type { z } from 'zod';
import type * as D from './dto';
import type { DebugCommand } from './commands';

export type ProviderKind = 'http' | 'fixture' | 'stand-in';
/** What the UI must declare at all times (spec 25.1: fixtures/stand-ins are labelled, never presented as real). */
export interface ModeDeclaration { provider: ProviderKind; target: string; runtime_profile: string; doubles: string[] }

export interface RunsQuery {
  cursor?: string; limit?: number; tenant?: string; environment?: string; state?: string; trigger?: string; source?: string; since?: string; until?: string;
}
export type RunList = z.infer<typeof D.RunList>;
export type Graph = z.infer<typeof D.Graph>;
export type DebugSession = z.infer<typeof D.Session>;
export type CommandAck = z.infer<typeof D.CommandAck>;

export interface StreamHandlers {
  /** Each event is delivered once, in sequence order, for this run only. */
  onEvent: (e: D.RunEvent) => void;
  /** 410 cursor_expired: replace the projection with `snapshot`; streaming resumes after `floor`. Nothing is invented for the purged range. */
  onReset: (r: { snapshot: Graph; floor: number; snapshotRef: string }) => void;
  onOpen: (reconnect: boolean) => void;
  onDrop: (attempt: number) => void;
  /** Any bytes received (events or heartbeat comments). */
  onActivity?: () => void;
  /** Terminal: session expired or no access (403/404 are indistinguishable). */
  onFatal: (reason: 'session_expired' | 'forbidden') => void;
}
export interface StreamOptions { afterSequence: number; random?: () => number }

/** The data-access port of the console. Screens depend on this interface only; providers are chosen once at boot. */
export interface DebugApi {
  readonly provider: ProviderKind;
  mode(): Promise<ModeDeclaration>;
  session(): Promise<DebugSession>;
  runs(q?: RunsQuery): Promise<RunList>;
  graph(runId: string): Promise<Graph>;
  events(runId: string, afterSequence: number): Promise<z.infer<typeof D.EventPage>>;
  modelCalls(runId: string): Promise<z.infer<typeof D.ModelCallPage>>;
  queries(runId: string): Promise<z.infer<typeof D.QueryPage>>;
  evals(runId: string): Promise<z.infer<typeof D.EvalPage>>;
  memoryDiff(runId: string): Promise<z.infer<typeof D.MemoryDiff>>;
  externalCommands(runId: string): Promise<z.infer<typeof D.ExternalCommandPage>>;
  dependencyHealth(): Promise<z.infer<typeof D.HealthPage>>;
  /** SSE with after_sequence / Last-Event-ID resume, dedup by (run, sequence) and 410 recovery. Returns stop(). */
  openStream(runId: string, handlers: StreamHandlers, opts: StreamOptions): () => void;
  /** Sends the command; the caller is responsible for gating the control with `commandState`. The server re-checks. */
  command(c: DebugCommand): Promise<CommandAck>;
}
