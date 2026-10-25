/** Text that attaches to the word before it, so no space goes in first. */
const ATTACHES_LEFT = /^[.,;:!?)]/;

/**
 * Put a dictated final into the composer at its caret or selection as they are
 * now, not where they were when dictation started, so text typed meanwhile is
 * kept. A single space separates it from a preceding non-space character.
 */
export function insertDictation(field: HTMLTextAreaElement, text: string) {
  const spoken = text.trim();
  if (!spoken) return;
  const start = field.selectionStart ?? field.value.length;
  const end = field.selectionEnd ?? start;
  const before = field.value.slice(0, start);
  const needsSpace =
    before.length > 0 && !/\s$/.test(before) && !ATTACHES_LEFT.test(spoken);
  const inserted = needsSpace ? ` ${spoken}` : spoken;
  field.setRangeText(inserted, start, end, "end");
  // Set explicitly: happy-dom's "end" mode puts the caret at the end of the
  // whole value rather than after the inserted text.
  const caret = start + inserted.length;
  field.setSelectionRange(caret, caret);
  // A raw `input` event is what React listens for.
  field.dispatchEvent(new Event("input", { bubbles: true }));
}
