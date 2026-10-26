import type { AgentField } from "./board";

/** What the app knows of one linked session; never stored. */
export type AgentFacts = {
  provider: string;
  model: string;
  /** Wall time from the session's start to its last update. */
  runTimeMs?: number;
  costUsd?: number;
};

export type AgentValues = Record<AgentField, string | null>;

const unique = (list: readonly string[]) => [...new Set(list)];

export function formatRunTime(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return "<1m";
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${rest}m`;
  return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
}

export function formatCost(usd: number): string {
  return usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

/**
 * A card's agent fields from the sessions linked to it: providers and models
 * joined, run time and cost summed. A field nothing can answer is null.
 */
export function agentValues(sessions: readonly AgentFacts[]): AgentValues {
  const times = sessions.flatMap((s) => (s.runTimeMs === undefined ? [] : [s.runTimeMs]));
  const costs = sessions.flatMap((s) => (s.costUsd === undefined ? [] : [s.costUsd]));
  return {
    provider: sessions.length ? unique(sessions.map((s) => s.provider)).join(", ") : null,
    model: sessions.length ? unique(sessions.map((s) => s.model)).join(", ") : null,
    cost: costs.length ? formatCost(costs.reduce((a, b) => a + b, 0)) : null,
    runTime: times.length ? formatRunTime(times.reduce((a, b) => a + b, 0)) : null,
  };
}

export const AGENT_FIELD_LABEL: Record<AgentField, string> = {
  provider: "Provider",
  model: "Model",
  cost: "Cost",
  runTime: "Run time",
};
