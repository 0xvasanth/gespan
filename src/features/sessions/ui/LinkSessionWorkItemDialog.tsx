import { useEffect, useRef, useState, type FormEvent } from "react";
import { Modal } from "../../../shared/ui/Modal";
import type { LinkedWorkItem, TicketTracker } from "../model/session";
import { parseGithubWorkItemUrl } from "../model/sessionWorkItem";
import {
  errorMessage,
  resolveTicket,
  ticketFromKey,
  trackerLabel,
} from "../model/ticketLinks";

export function LinkSessionWorkItemDialog({
  initial,
  sessionTitle,
  ticketOnly = false,
  heading,
  onSave,
  onClose,
}: {
  initial?: LinkedWorkItem;
  sessionTitle: string;
  /** Takes a ticket only (a board card's link): a GitHub link is refused. */
  ticketOnly?: boolean;
  /** Replaces the default title. */
  heading?: string;
  onSave: (item: LinkedWorkItem | undefined) => void;
  onClose: () => void;
}) {
  const [url, setUrl] = useState(() => {
    if (!initial) return "";
    return initial.kind === "ticket"
      ? (initial.ticket.url ?? initial.ticket.key)
      : initial.url;
  });
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  // Bumped whenever an in-flight resolve stops being wanted: the input
  // changed, the dialog was cancelled or removed the link, or it unmounted.
  const request = useRef(0);
  useEffect(
    () => () => {
      request.current += 1;
    },
    [],
  );
  const cancelPending = () => {
    request.current += 1;
    setBusy(false);
  };
  const [choice, setChoice] = useState<{
    key: string;
    trackers: TicketTracker[];
  } | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const input = url.trim();
    if (!input || busy) return;
    const github = ticketOnly ? null : parseGithubWorkItemUrl(input);
    if (github) {
      onSave(github);
      return;
    }
    const mine = ++request.current;
    setBusy(true);
    setChoice(null);
    try {
      const result = await resolveTicket(input);
      if (mine !== request.current) return;
      if (result.kind === "one") {
        onSave({ kind: "ticket", ticket: result.ticket });
      } else if (result.kind === "ambiguous") {
        setChoice({ key: result.key, trackers: result.trackers });
      } else {
        setError(result.message);
      }
    } catch (err) {
      if (mine === request.current) setError(errorMessage(err));
    } finally {
      if (mine === request.current) setBusy(false);
    }
  };

  return (
    <Modal
      title={heading ?? (initial ? "Edit link" : "Link issue, PR or ticket")}
      description={sessionTitle}
      size="sm"
      onClose={onClose}
    >
      <form
        onSubmit={(event) => void submit(event)}
        className="flex flex-col gap-4 p-4 text-[12px]"
      >
        <label className="flex flex-col gap-1.5">
          <span className="font-medium text-content/80">
            {ticketOnly ? "Ticket" : "Issue, PR or ticket"}
          </span>
          <input
            autoFocus
            type="text"
            value={url}
            aria-label={
              ticketOnly
                ? "Ticket link or key"
                : "Issue, PR or ticket link or key"
            }
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? "linked-work-item-error" : undefined}
            placeholder={
              ticketOnly
                ? "Ticket link or key like ENG-123"
                : "GitHub link, ticket link or key like ENG-123"
            }
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => {
              setUrl(event.target.value);
              if (busy) cancelPending();
              if (error) setError("");
              if (choice) setChoice(null);
            }}
            className={`h-9 rounded-md border bg-content/5 px-2.5 text-[13px] text-content outline-none placeholder:text-content/30 focus:border-content/30 ${
              error ? "border-red-400/60" : "border-content/10"
            }`}
          />
          {error ? (
            <span
              id="linked-work-item-error"
              role="alert"
              className="text-[11px] text-red-400"
            >
              {error}
            </span>
          ) : (
            <span className="text-[11px] text-content/45">
              {ticketOnly
                ? "Paste a Jira, Linear or Plane link, or a ticket key."
                : "Paste a GitHub link, a Jira, Linear or Plane link, or a ticket key. The link appears on the session."}
            </span>
          )}
        </label>
        {choice ? (
          <div className="flex flex-col gap-1.5">
            <span className="text-content/65">
              Which tracker is {choice.key} in?
            </span>
            <div className="flex gap-2">
              {choice.trackers.map((tracker) => (
                <button
                  key={tracker}
                  type="button"
                  data-tracker-choice={tracker}
                  onClick={() =>
                    onSave({
                      kind: "ticket",
                      ticket: ticketFromKey(tracker, choice.key),
                    })
                  }
                  className="rounded-md border border-content/10 px-3 py-1.5 hover:bg-content/8 active:scale-[0.97]"
                >
                  {trackerLabel(tracker)}
                </button>
              ))}
            </div>
          </div>
        ) : null}
        <div className="flex items-center justify-end gap-2">
          {initial ? (
            <button
              type="button"
              onClick={() => {
                cancelPending();
                onSave(undefined);
              }}
              className="mr-auto rounded-md px-3 py-1.5 text-red-400 hover:bg-red-400/10 active:scale-[0.97]"
            >
              Remove link
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => {
              cancelPending();
              onClose();
            }}
            className="rounded-md px-3 py-1.5 hover:bg-content/8 active:scale-[0.97]"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={busy}
            className="rounded-md bg-accent px-3 py-1.5 font-medium text-white hover:brightness-110 active:scale-[0.97]"
          >
            {busy ? "Checking" : initial ? "Update link" : "Link"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
