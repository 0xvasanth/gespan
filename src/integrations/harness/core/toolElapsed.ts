/**
 * Live elapsed seconds for running tool calls, taken from Claude's ~30 s
 * heartbeat frames. Held in memory only: it is never part of a session, so it
 * is never saved, and the shared rebase.db format is unchanged.
 */
const elapsed = new Map<string, number>();
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of [...listeners]) listener();
}

export function setToolElapsed(callId: string, seconds: number): void {
  if (elapsed.get(callId) === seconds) return;
  elapsed.set(callId, seconds);
  emit();
}

export function clearToolElapsed(callId: string): void {
  if (elapsed.delete(callId)) emit();
}

export function clearAllToolElapsed(): void {
  if (!elapsed.size) return;
  elapsed.clear();
  emit();
}

export function toolElapsed(callId: string | undefined): number | undefined {
  return callId === undefined ? undefined : elapsed.get(callId);
}

export function subscribeToolElapsed(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** `Ns` under a minute, `Mm Ss` after. Same as Rebase Native. */
export function formatElapsed(seconds: number): string {
  const secs = Math.max(0, Math.floor(seconds));
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m ${secs % 60}s`;
}
