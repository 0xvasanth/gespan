/** Board keyboard shortcuts: what a keydown means, decided without the DOM tree of the board. */

const EDITABLE_TAGS = new Set(["INPUT", "TEXTAREA", "SELECT"]);

const EDITABLE_SELECTOR = [
  "input",
  "textarea",
  "select",
  '[contenteditable=""]',
  '[contenteditable="true"]',
  '[contenteditable="plaintext-only"]',
  '[role="textbox"]',
  '[role="searchbox"]',
  '[role="combobox"]',
  ".cm-editor",
  ".cm-content",
].join(",");

/** True when typing into `target` must not trigger a shortcut. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof Element)) return false;
  if (EDITABLE_TAGS.has(target.tagName)) return true;
  if ((target as HTMLElement).isContentEditable) return true;
  return target.closest(EDITABLE_SELECTOR) !== null;
}

export type BoardShortcut = "new" | "search" | "open" | "delete";

const CARD_SELECTOR = "[data-kanban-card], [data-card-id], [data-table-row]";

/** The card element a focused target belongs to, if it is the card itself or plain content of it. */
export function focusedCardId(target: EventTarget | null): string | null {
  if (!(target instanceof Element)) return null;
  const card = target.closest(CARD_SELECTOR);
  if (!card) return null;
  // A control inside the card (a menu button, say) keeps its own keys.
  const control = target.closest('button, a, [role="menu"], [role="menuitem"]');
  if (control && control !== card) return null;
  return (
    card.getAttribute("data-kanban-card") ??
    card.getAttribute("data-card-id") ??
    card.getAttribute("data-table-row")
  );
}

export function shortcutFor(event: {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  isComposing?: boolean;
  repeat?: boolean;
  defaultPrevented?: boolean;
}): BoardShortcut | null {
  if (
    event.metaKey ||
    event.ctrlKey ||
    event.altKey ||
    event.isComposing ||
    event.defaultPrevented
  )
    return null;
  switch (event.key) {
    case "n":
    case "N":
      return event.repeat ? null : "new";
    case "/":
      return "search";
    case "e":
    case "E":
      return event.repeat ? null : "open";
    case "Delete":
    case "Backspace":
      return event.repeat ? null : "delete";
    default:
      return null;
  }
}
