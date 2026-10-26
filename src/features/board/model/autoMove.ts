import {
  boardLinkedEvent,
  type CardLinkKind,
  type LinkedWorkEvent,
} from "./board";

/**
 * Applies the boards' auto-move rules to every card linked to this work. The
 * rules and the move live in the shared board store (the native app runs the
 * same engine): each card moves per its board's rule (forward-only by
 * default) and records the event, one transaction per card. One failing card
 * does not stop the others; the first error rejects.
 */
export function onLinkedWork(
  kind: CardLinkKind,
  id: string,
  event: LinkedWorkEvent,
): Promise<void> {
  return boardLinkedEvent(kind, id, event);
}
