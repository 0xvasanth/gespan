import { onTestFinished, vi } from "vitest";

/**
 * Deliver `selectionchange` after the current task, as browsers do.
 *
 * happy-dom dispatches it synchronously from inside `Selection.collapse`, so
 * when CodeMirror writes the DOM selection during `EditorView.update`, its
 * selection observer re-enters `update` and throws "Calls to
 * EditorView.update are not allowed while an update is in progress".
 *
 * happy-dom also calls `dispatchEvent` again on the same target while walking
 * the capture and bubble phases, so only the first dispatch of an event is
 * deferred; re-queueing the inner calls would never let the event finish.
 */
export function queueSelectionChange(doc: Document = document) {
  const dispatch = doc.dispatchEvent.bind(doc);
  const queued = new WeakSet<Event>();
  const spy = vi.spyOn(doc, "dispatchEvent").mockImplementation((event) => {
    if (event.type !== "selectionchange" || queued.has(event)) {
      return dispatch(event);
    }
    queued.add(event);
    queueMicrotask(() => dispatch(event));
    return true;
  });
  onTestFinished(() => spy.mockRestore());
}
