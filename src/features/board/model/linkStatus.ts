import type { CardLinkKind, CardLinkRow } from "./board";

/** What one link on a card is doing. */
export type LinkState =
  | "running"
  | "waiting"
  | "failed"
  | "completed"
  | "idle"
  | "open"
  | "merged"
  | "closed"
  | "unknown"
  | "missing";

export type ChecksState = "passing" | "failing" | "pending";

/** What the app finds out about a PR or worktree link, off the render path. */
export type LinkExtra = {
  state?: LinkState;
  checks?: ChecksState;
  /** Worktree: the branch, and lines added and removed. */
  branch?: string;
  diff?: { additions: number; deletions: number };
};

/** One chip on a card: one per link. */
export type LinkChipView = {
  kind: CardLinkKind | "pr" | "worktree";
  reference: string;
  label: string;
  state: LinkState;
  checks?: ChecksState;
  diff?: { additions: number; deletions: number };
};

/** A session's chip state: running beats waiting beats failed beats completed. */
export function sessionState(input: {
  running: boolean;
  waiting: boolean;
  /** The latest linked-work event for the card is `failed`. */
  failed: boolean;
}): LinkState {
  if (input.running) return "running";
  if (input.waiting) return "waiting";
  if (input.failed) return "failed";
  return "completed";
}

/** A Pilot is running or idle. */
export function pilotState(running: boolean): LinkState {
  return running ? "running" : "idle";
}

/** A PR's `gh` state (OPEN, MERGED, CLOSED, any case); anything else is unknown. */
export function prState(state: string | null | undefined): LinkState {
  switch (state?.toLowerCase()) {
    case "open":
      return "open";
    case "merged":
      return "merged";
    case "closed":
      return "closed";
    default:
      return "unknown";
  }
}

/** Failing beats pending beats passing; nothing to report is unknown (undefined). */
export function checksState(
  checks: readonly { state: string }[],
): ChecksState | undefined {
  const real = checks.filter(
    (check) => check.state !== "skipping" && check.state !== "cancel",
  );
  if (real.some((check) => check.state === "fail")) return "failing";
  if (real.some((check) => check.state === "pending" || check.state === "unknown")) {
    return "pending";
  }
  return real.length ? "passing" : undefined;
}

/** `https://github.com/{owner}/{repo}/pull/{n}` as repo and number. */
export function parsePrUrl(
  url: string,
): { repo: string; number: number } | null {
  const match =
    /^https:\/\/github\.com\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)\/pull\/(\d+)\/?(?:[?#].*)?$/.exec(
      url,
    );
  if (!match) return null;
  const number = Number(match[3]);
  return Number.isSafeInteger(number) && number > 0
    ? { repo: `${match[1]}/${match[2]}`, number }
    : null;
}

/** The text of a chip's status, for its tooltip and reading. */
export function chipStatusText(chip: LinkChipView): string {
  const parts: string[] = [chip.state];
  if (chip.checks) parts.push(`checks ${chip.checks}`);
  if (chip.diff) parts.push(`+${chip.diff.additions} −${chip.diff.deletions}`);
  return parts.join(", ");
}

/** The pairs `getRunningCards` is asked about: linked work with an open turn (running or waiting). */
export function runningPairs(
  targets: readonly { kind: CardLinkKind; id: string; status?: string }[],
): [CardLinkKind, string][] {
  return targets
    .filter(
      (target) =>
        target.kind !== "flow-run" &&
        // A turn waiting on the user is still an open turn.
        (target.status === "running" || target.status === "waiting"),
    )
    .map((target): [CardLinkKind, string] => [target.kind, target.id])
    .sort((a, b) => (a.join() < b.join() ? -1 : 1));
}

/** Whether something that was running no longer is: a linked turn settled. */
export function settledSince(
  before: readonly [CardLinkKind, string][],
  after: readonly [CardLinkKind, string][],
): boolean {
  const now = new Set(after.map((pair) => pair.join("\n")));
  return before.some((pair) => !now.has(pair.join("\n")));
}

/** Keys of the extras a link needs looked up (PRs and worktrees). */
export function extraKey(link: Pick<CardLinkRow, "kind" | "reference">): string {
  return `${link.kind}\n${link.reference}`;
}
