// Commands of spec 25 ("Comandos de depuracion autorizados"). Authority is never derived from a role label:
// a command is enabled only when the SERVER lists it in `available_commands[]` AND the session scope allows it.
export type CommandName = 'pause' | 'resume' | 'cancel' | 'retry' | 'fork_replay';

interface Base { expectedRevision: number; idempotencyKey: string; reason: string }
export type DebugCommand =
  | (Base & { kind: 'pause' | 'resume' | 'cancel' | 'fork_replay'; runId: string })
  | (Base & { kind: 'retry'; jobId: string });

/** Provisional scope names (the spec only names roles; the control-api owner must confirm). One scope per command. */
export const SCOPE_FOR_COMMAND: Record<CommandName, string> = {
  pause: 'debug:command:pause', resume: 'debug:command:resume', cancel: 'debug:command:cancel',
  retry: 'debug:command:retry', fork_replay: 'debug:command:fork_replay',
};
const isCommand = (c: string): c is CommandName => Object.prototype.hasOwnProperty.call(SCOPE_FOR_COMMAND, c);

export type CommandState = { enabled: true } | { enabled: false; reason: 'not_offered' | 'scope_missing' | 'unknown_command' };

export function commandState(command: string, availableCommands: readonly string[], scopes: readonly string[]): CommandState {
  if (!isCommand(command)) return { enabled: false, reason: 'unknown_command' };
  if (!availableCommands.includes(command)) return { enabled: false, reason: 'not_offered' };
  if (!scopes.includes(SCOPE_FOR_COMMAND[command])) return { enabled: false, reason: 'scope_missing' };
  return { enabled: true };
}

export const commandPath = (c: DebugCommand, base: string): string => {
  switch (c.kind) {
    case 'retry': return `${base}/jobs/${encodeURIComponent(c.jobId)}/retry`;
    case 'fork_replay': return `${base}/runs/${encodeURIComponent(c.runId)}/fork-replay`;
    default: return `${base}/runs/${encodeURIComponent(c.runId)}/${c.kind}`;
  }
};
