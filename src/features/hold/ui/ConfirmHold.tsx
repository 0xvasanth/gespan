import { Modal } from "../../../shared/ui/Modal";

/** Asked before Hold starts; it can't be undone until every session stops. */
export function ConfirmHold({
  running,
  onClose,
}: {
  running: number;
  onClose: (hold: boolean) => void;
}) {
  const sessions = running === 1 ? "1 running session" : `${running} running sessions`;
  return (
    <Modal title="Hold everything?" size="sm" onClose={() => onClose(false)}>
      <div className="flex flex-col gap-4 p-4 text-[12px]">
        <p>
          {sessions} will stop once their current command finishes, and nothing
          new will start. Messages you send will wait until you resume. You
          can't resume until every session has stopped.
        </p>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={() => onClose(false)}
            className="rounded-md px-3 py-1.5 hover:bg-content/8 active:scale-[0.97]"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onClose(true)}
            className="rounded-md bg-accent px-3 py-1.5 font-medium text-white hover:brightness-110 active:scale-[0.97]"
          >
            Hold
          </button>
        </div>
      </div>
    </Modal>
  );
}
