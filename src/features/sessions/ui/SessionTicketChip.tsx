import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertCircle, MoreHorizontal } from "../../../shared/ui/icons";
import type { LinkedWorkItem, TicketRef } from "../model/session";
import {
  errorMessage,
  refreshLinkedTicket,
  statusGroupTone,
  trackerLabel,
  useConnectedTrackers,
} from "../model/ticketLinks";

const FRESH_MS = 60_000;

/**
 * The ticket a session is linked to. Refreshes when the session is opened or
 * on request, never on a timer.
 */
export function SessionTicketChip({
  ticket,
  visible,
  onChange,
  current,
}: {
  ticket: TicketRef;
  /** The session is open in a pane. */
  visible: boolean;
  /** Save a new link, or `undefined` to unlink. */
  onChange: (item: LinkedWorkItem | undefined) => void;
  /** The session's link right now, to drop a fetch that lost the race. */
  current: () => LinkedWorkItem | undefined;
}) {
  const connected = useConnectedTrackers();
  const [menuOpen, setMenuOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const root = useRef<HTMLSpanElement>(null);
  const latest = useRef({ ticket, onChange, current });
  latest.current = { ticket, onChange, current };

  const label = trackerLabel(ticket.tracker);
  const stale = connected !== null && !connected.has(ticket.tracker);

  const refresh = useCallback(async () => {
    const { ticket, onChange, current } = latest.current;
    setRefreshing(true);
    setError(null);
    try {
      await refreshLinkedTicket({ ticket, current, save: onChange });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    if (!visible) return;
    const fetchedAt = latest.current.ticket.fetchedAt;
    // Switching tabs should not hammer the tracker.
    if (fetchedAt != null && Date.now() - fetchedAt < FRESH_MS) return;
    void refresh();
  }, [visible, refresh]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = (event: Event) => {
      if (event.target instanceof Node && root.current?.contains(event.target))
        return;
      setMenuOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  const menu: Array<[string, () => void, boolean?]> = [
    [
      `Open in ${label}`,
      () => {
        if (ticket.url) void openUrl(ticket.url).catch(() => undefined);
      },
      !ticket.url,
    ],
    ["Refresh", () => void refresh(), refreshing],
    ["Unlink", () => onChange(undefined)],
  ];

  return (
    <span
      ref={root}
      className="relative inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-md border border-content/10 bg-content/5 py-0.5 pr-0.5 pl-2 text-[11px] text-content/80"
    >
      <span className="shrink-0 text-content/45">{label}</span>
      <span className="shrink-0 font-medium tabular-nums text-content">
        {ticket.key}
      </span>
      {ticket.title ? (
        <span className="min-w-0 max-w-[28ch] truncate" title={ticket.title}>
          {ticket.title}
        </span>
      ) : null}
      {ticket.status ? (
        <span
          data-status-group={ticket.statusGroup ?? "unknown"}
          className={`shrink-0 rounded-full px-1.5 py-px text-[10px] ${statusGroupTone(ticket.statusGroup)}`}
        >
          {ticket.status}
        </span>
      ) : null}
      {stale ? (
        <span
          data-stale="true"
          title={`${label} is disconnected. Reconnect in Settings.`}
          className="shrink-0 text-amber-400"
        >
          <AlertCircle className="size-3" strokeWidth={1.75} />
        </span>
      ) : null}
      {error ? (
        <span role="status" className="sr-only">
          {error}
        </span>
      ) : null}
      <button
        type="button"
        aria-label="Ticket actions"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        title={error ?? "Ticket actions"}
        onClick={() => setMenuOpen((open) => !open)}
        className="grid size-5 shrink-0 place-items-center rounded text-content/50 hover:bg-content/10 hover:text-content"
      >
        <MoreHorizontal className="size-3.5" strokeWidth={1.75} />
      </button>
      {menuOpen ? (
        <div
          role="menu"
          className="absolute top-full right-0 z-20 mt-1 flex min-w-36 flex-col rounded-md border border-content/10 bg-background-base p-1 shadow-lg"
        >
          {menu.map(([text, run, disabled]) => (
            <button
              key={text}
              type="button"
              role="menuitem"
              disabled={disabled}
              onClick={() => {
                setMenuOpen(false);
                run();
              }}
              className="rounded px-2 py-1 text-left hover:bg-content/8 disabled:opacity-40"
            >
              {text}
            </button>
          ))}
        </div>
      ) : null}
    </span>
  );
}
