import { invoke } from "@tauri-apps/api/core";
import { useEffect, useSyncExternalStore } from "react";
import type {
  LinkedWorkItem,
  TicketRef,
  TicketStatusGroup,
  TicketTracker,
} from "./session";

export type TicketResolveResult =
  | { kind: "one"; ticket: TicketRef }
  | { kind: "ambiguous"; key: string; trackers: TicketTracker[] }
  | { kind: "error"; message: string };

const TRACKER_LABELS: Record<TicketTracker, string> = {
  jira: "Jira",
  linear: "Linear",
  plane: "Plane",
};

export function trackerLabel(tracker: TicketTracker): string {
  return TRACKER_LABELS[tracker];
}

export function ticketFromKey(tracker: TicketTracker, key: string): TicketRef {
  return {
    tracker,
    key,
    url: null,
    title: null,
    status: null,
    statusGroup: null,
    fetchedAt: null,
  };
}

export function errorMessage(error: unknown): string {
  return String(error instanceof Error ? error.message : error);
}

export function resolveTicket(input: string): Promise<TicketResolveResult> {
  return invoke<TicketResolveResult>("ticket_resolve", { input });
}

const inflightFetches = new Map<string, Promise<TicketRef>>();

/** Concurrent fetches of one ticket share a single request. */
export function fetchTicket(ticket: TicketRef): Promise<TicketRef> {
  const id = `${ticket.tracker}:${ticket.key}`;
  const pending = inflightFetches.get(id);
  if (pending) return pending;
  const request = invoke<TicketRef>("ticket_fetch", { ticket }).finally(() => {
    if (inflightFetches.get(id) === request) inflightFetches.delete(id);
  });
  inflightFetches.set(id, request);
  return request;
}

export function sameTicket(a: TicketRef, b: TicketRef): boolean {
  return a.tracker === b.tracker && a.key === b.key;
}

/**
 * Fetch a ticket and hand the filled copy to `save`, unless the session was
 * linked to something else while the request was out.
 */
export async function refreshLinkedTicket(options: {
  ticket: TicketRef;
  current: () => LinkedWorkItem | undefined;
  save: (item: LinkedWorkItem) => void;
}): Promise<TicketRef> {
  const filled = await fetchTicket(options.ticket);
  const now = options.current();
  if (now?.kind === "ticket" && sameTicket(now.ticket, options.ticket)) {
    options.save({ kind: "ticket", ticket: filled });
  }
  return filled;
}

export function statusGroupTone(group: TicketStatusGroup | null): string {
  switch (group) {
    case "started":
      return "bg-amber-400/15 text-amber-400";
    case "completed":
      return "bg-emerald-400/15 text-emerald-400";
    case "cancelled":
      return "bg-content/10 text-content/45";
    case "triage":
    case "backlog":
    case "unstarted":
      return "bg-sky-400/15 text-sky-400";
    default:
      return "bg-content/10 text-content/60";
  }
}

// Which trackers have saved credentials. Loaded once on first use and again
// when a connection changes, never per render.
export const TICKET_CONNECTIONS_EVENT = "gespan:ticket-connections-change";

let connected: ReadonlySet<TicketTracker> | null = null;
let loading = false;
let generation = 0;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

async function loadConnected() {
  if (loading) return;
  loading = true;
  const mine = ++generation;
  try {
    const trackers = await invoke<TicketTracker[]>("ticket_connected");
    if (mine === generation) connected = new Set(trackers);
  } catch {
    // Unknown: leave it unset so no chip claims a tracker is disconnected.
  } finally {
    loading = false;
  }
  emit();
}

export function notifyTicketConnectionsChanged() {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new Event(TICKET_CONNECTIONS_EVENT));
}

function reloadConnected() {
  generation += 1;
  loading = false;
  void loadConnected();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    window.addEventListener(TICKET_CONNECTIONS_EVENT, reloadConnected);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      window.removeEventListener(TICKET_CONNECTIONS_EVENT, reloadConnected);
    }
  };
}

/** `null` until known. */
export function useConnectedTrackers(): ReadonlySet<TicketTracker> | null {
  const value = useSyncExternalStore(
    subscribe,
    () => connected,
    () => null,
  );
  useEffect(() => {
    if (connected === null) void loadConnected();
  }, []);
  return value;
}

export function resetTicketLinkCaches() {
  connected = null;
  loading = false;
  generation += 1;
  inflightFetches.clear();
}
