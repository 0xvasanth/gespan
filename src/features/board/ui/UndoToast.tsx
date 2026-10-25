import { LAYER } from "../../../shared/lib/layers";
import type { UndoToast as Toast } from "../model/undo";

/**
 * The board's undo toast: one at a time, bottom centre, so it never covers
 * the error toast at the bottom right. Undo shows only for an undoable action.
 */
export function UndoToast({
  toast,
  onUndo,
}: {
  toast: Toast | null;
  onUndo: () => void;
}) {
  if (!toast) return null;
  return (
    <div
      className="pointer-events-none fixed inset-x-0 bottom-4 flex justify-center"
      style={{ zIndex: LAYER.toast }}
    >
      <div
        key={toast.id}
        role="status"
        aria-live="polite"
        data-board-undo-toast
        className="pointer-events-auto flex w-[340px] max-w-[calc(100vw-32px)] items-center gap-2 rounded-xl border border-content/15 bg-[#252525] px-3 py-2 text-[12px] text-content shadow-xl"
      >
        <span className="min-w-0 flex-1 truncate">{toast.label}</span>
        {toast.action ? (
          <button
            type="button"
            onClick={onUndo}
            className="h-6 shrink-0 rounded-md px-2.5 text-accent hover:bg-content/10"
          >
            Undo
          </button>
        ) : null}
      </div>
    </div>
  );
}
