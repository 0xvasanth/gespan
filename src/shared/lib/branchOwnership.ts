/** Prefix for branches that Rebase creates itself. */
export const APP_BRANCH_PREFIX = "rebase/";
/** Legacy prefixes, still recognised so existing worktrees keep working. */
export const LEGACY_BRANCH_PREFIXES = ["mc/"] as const;

const OWNED_PREFIXES: readonly string[] = [APP_BRANCH_PREFIX, ...LEGACY_BRANCH_PREFIXES];

export const isAppBranch = (name: string | null | undefined): boolean =>
  !!name && OWNED_PREFIXES.some((prefix) => name.startsWith(prefix));

export const isOrchestrationBranch = (name: string | null | undefined): boolean =>
  !!name && OWNED_PREFIXES.some((prefix) => name.startsWith(`${prefix}orch-`));

/** The 8-character token branch Rebase gives a fresh worktree before it is renamed. */
export const isTemporaryWorktreeBranch = (name: string): boolean =>
  OWNED_PREFIXES.some(
    (prefix) => name.startsWith(prefix) && /^[a-z0-9]{8}$/.test(name.slice(prefix.length)),
  );
