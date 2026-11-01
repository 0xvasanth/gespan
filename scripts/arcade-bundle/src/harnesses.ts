// The logos the arcade games can show.
export const HARNESSES = ["claude", "codex", "cursor", "grok", "opencode", "pi", "omp", "fx", "hermes", "antigravity", "devin"] as const;
export type HarnessId = (typeof HARNESSES)[number];
