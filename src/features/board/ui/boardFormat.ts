import type {
  BoardActivity,
  BoardCard,
  CardLinkKind,
  ColumnRole,
} from "../model/board";

export const ROLE_LABEL: Record<ColumnRole, string> = {
  todo: "To do",
  doing: "In progress",
  review: "Review",
  done: "Done",
};

export const ROLES: readonly ColumnRole[] = ["todo", "doing", "review", "done"];

/** Something a card can link to, as the app currently knows it. */
export type BoardLinkTarget = {
  kind: CardLinkKind;
  id: string;
  label: string;
  /** Shown after the label in the picker, e.g. the project. */
  detail?: string;
  status?: "running" | "completed" | "failed";
};

export const LINK_KIND_LABEL: Record<CardLinkKind, string> = {
  session: "Session",
  pilot: "Pilot",
  "flow-run": "Flow run",
};

/** A card's linked work: the target, or `null` when it no longer exists. */
export type ResolvedLink = {
  kind: CardLinkKind;
  target: BoardLinkTarget | null;
};

export function resolveLink(
  card: Pick<BoardCard, "linkKind" | "linkId">,
  targets: ReadonlyMap<string, BoardLinkTarget>,
): ResolvedLink | null {
  if (!card.linkKind || !card.linkId) return null;
  return {
    kind: card.linkKind,
    target: targets.get(linkValue(card.linkKind, card.linkId)) ?? null,
  };
}

export function linkValue(kind: CardLinkKind, id: string): string {
  return `${kind}:${id}`;
}

export function parseLinkValue(
  value: string,
): { kind: CardLinkKind; id: string } | null {
  const at = value.indexOf(":");
  if (at < 0) return null;
  const kind = value.slice(0, at);
  const id = value.slice(at + 1);
  if (!id) return null;
  if (kind === "session" || kind === "pilot" || kind === "flow-run") {
    return { kind, id };
  }
  return null;
}

/** A command failure as text: Tauri rejects with strings, JS with Errors. */
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Compact time since `ms`: "just now", "2m ago", "3h ago", "4d ago". */
export function relativeTime(ms: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 45) return "just now";
  const minutes = Math.max(1, Math.round(seconds / 60));
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 7) return `${days}d ago`;
  if (days < 30) return `${Math.round(days / 7)}w ago`;
  return new Date(ms).toLocaleDateString();
}

function activityAction(row: BoardActivity): string {
  switch (row.kind) {
    case "created":
      return row.toColumn ? `Created in ${row.toColumn}` : "Created";
    case "edited":
      return "Edited";
    case "moved":
      return `Moved from ${row.fromColumn ?? "?"} to ${row.toColumn ?? "?"}`;
    case "linked":
      return "Linked work";
    case "unlinked":
      return "Unlinked work";
    case "deleted":
      return "Deleted";
    case "started":
      return "Linked work started";
    case "completed":
      return "Linked work completed";
    case "failed":
      return "Linked work failed";
  }
}

/** One timeline row: "Moved from In Progress to Review · system · 2m ago". */
export function activityText(row: BoardActivity, now = Date.now()): string {
  return `${activityAction(row)} · ${row.actor} · ${relativeTime(row.createdAt, now)}`;
}
