/**
 * Claude's keep-alive for a running tool call:
 * `{"type":"tool_progress","tool_use_id":"<P>-heartbeat-<n>",
 *   "parent_tool_use_id":"<P>","heartbeat":true,"elapsed_time_seconds":…}`.
 * Same definition as Gespan Native (`gespan-core/src/tool/tasks.rs`).
 */
const HEARTBEAT_SUFFIX = /-heartbeat-(\d+)$/;

function split(id: string): { parent: string; n: number } | undefined {
  const match = HEARTBEAT_SUFFIX.exec(id);
  if (!match || match.index === 0) return undefined;
  return { parent: id.slice(0, match.index), n: Number(match[1]) };
}

/** A saved step or call that Claude's keep-alive created, not real work. */
export function isHeartbeatId(id: string): boolean {
  return split(id) !== undefined;
}

export type HeartbeatShape = {
  toolUseId: string;
  parentToolUseId?: string;
  heartbeat?: boolean;
  elapsedSeconds?: number;
};

export function isHeartbeatProgress(progress: HeartbeatShape): boolean {
  if (progress.heartbeat === true) return true;
  const parts = split(progress.toolUseId);
  return !!parts && progress.parentToolUseId === parts.parent;
}

/** The running call a heartbeat is about, and how long it has run. */
export function heartbeatElapsed(
  progress: HeartbeatShape,
): { callId: string; seconds: number } | undefined {
  const parts = split(progress.toolUseId);
  const callId = progress.parentToolUseId ?? parts?.parent;
  if (!callId) return undefined;
  const seconds =
    progress.elapsedSeconds !== undefined &&
    Number.isFinite(progress.elapsedSeconds) &&
    progress.elapsedSeconds >= 0
      ? Math.floor(progress.elapsedSeconds)
      : parts
        ? 30 * (parts.n + 1)
        : undefined;
  return seconds === undefined ? undefined : { callId, seconds };
}
