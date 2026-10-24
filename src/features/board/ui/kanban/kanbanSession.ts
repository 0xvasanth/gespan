import type { BoardDetail } from "../../model/board";
import {
  applyCardMove,
  applyColumnMove,
  type DragKind,
  type DropRef,
  type DropTarget,
  finalCardTarget,
  resolveCardDrop,
  resolveColumnDrop,
} from "./kanbanMove";

export type ActiveDrag = { kind: DragKind; id: string };

/** The board's drag state, as pure transitions the component drives. */
export type DragSession = {
  /** Latest `detail` from the parent. */
  detail: BoardDetail;
  /** Drag preview or optimistic result; null shows `detail`. */
  local: BoardDetail | null;
  drag:
    | (ActiveDrag & {
        /** The board as shown when the drag started. */
        snapshot: BoardDetail;
        /** `detail` when the drag started, to spot refreshes mid-drag. */
        startDetail: BoardDetail;
        /**
         * The card that made the latest cross-column preview, while it is
         * still the latest `over`. A drop over it keeps the preview: the
         * card was shown before it, not sorted past it.
         */
        previewOver: string | null;
      })
    | null;
  /**
   * Snapshot of the most recent drag; kept after it ends so the end / cancel
   * announcements, which dnd-kit fires after our handlers, still see it.
   */
  lastSnapshot: BoardDetail | null;
};

export type DragCommit =
  | { kind: "card"; cardId: string; to: DropTarget }
  | { kind: "column"; columnId: string; index: number };

export function createSession(detail: BoardDetail): DragSession {
  return { detail, local: null, drag: null, lastSnapshot: null };
}

export function shownBoard(s: DragSession): BoardDetail {
  return s.local ?? s.detail;
}

/** A new `detail` drops the optimistic copy, unless a drag is in progress. */
export function sessionDetailChanged(
  s: DragSession,
  detail: BoardDetail,
): DragSession {
  if (s.detail === detail) return s;
  return { ...s, detail, local: s.drag ? s.local : null };
}

export function sessionStart(s: DragSession, drag: ActiveDrag): DragSession {
  const snapshot = shownBoard(s);
  return {
    ...s,
    drag: { ...drag, snapshot, startDetail: s.detail, previewOver: null },
    lastSnapshot: snapshot,
  };
}

/** Previews a card joining another column; same-column moves need none. */
export function sessionOver(s: DragSession, over: DropRef | null): DragSession {
  const drag = s.drag;
  if (drag?.kind !== "card") return s;
  if (!over) return withPreviewOver(s, null);
  if (over.overKind === "card" && over.overId === drag.previewOver) return s;
  const current = shownBoard(s);
  const card = current.cards.find((c) => c.id === drag.id);
  const target = resolveCardDrop(current, drag.id, over.overId, over.overKind);
  if (!card || !target || target.columnId === card.columnId) {
    return withPreviewOver(s, null);
  }
  return {
    ...s,
    local: applyCardMove(current, drag.id, target),
    drag: {
      ...drag,
      previewOver: over.overKind === "card" ? over.overId : null,
    },
  };
}

function withPreviewOver(s: DragSession, previewOver: string | null) {
  if (!s.drag || s.drag.previewOver === previewOver) return s;
  return { ...s, drag: { ...s.drag, previewOver } };
}

// If the parent refreshed mid-drag, the snapshot is stale: build on `detail`.
function freshBase(s: DragSession): BoardDetail | null {
  if (!s.drag) return null;
  return s.drag.startDetail === s.detail ? s.drag.snapshot : s.detail;
}

/** Ends the drag without a move: back to the board as it was at drag start. */
export function sessionCancel(s: DragSession): DragSession {
  const base = freshBase(s);
  if (!base) return s;
  return { ...s, drag: null, local: base === s.detail ? null : base };
}

/** Ends the drag; `commit` is the move to report, if any. */
export function sessionEnd(
  s: DragSession,
  over: DropRef | null,
): { session: DragSession; commit: DragCommit | null } {
  const drag = s.drag;
  const base = freshBase(s);
  if (!drag || !base) return { session: s, commit: null };
  const ended = { ...s, drag: null };
  if (drag.kind === "card") {
    const settled =
      over?.overKind === "card" && over.overId === drag.previewOver
        ? null
        : over;
    const to = finalCardTarget(drag.snapshot, shownBoard(s), drag.id, settled);
    if (to) {
      return {
        session: { ...ended, local: applyCardMove(base, drag.id, to) },
        commit: { kind: "card", cardId: drag.id, to },
      };
    }
  } else {
    const index =
      over && resolveColumnDrop(shownBoard(s), drag.id, over.overId);
    if (index != null) {
      return {
        session: { ...ended, local: applyColumnMove(base, drag.id, index) },
        commit: { kind: "column", columnId: drag.id, index },
      };
    }
  }
  return { session: sessionCancel(s), commit: null };
}
