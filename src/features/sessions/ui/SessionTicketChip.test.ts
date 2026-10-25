// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SessionTicketChip } from "./SessionTicketChip";
import {
  notifyTicketConnectionsChanged,
  resetTicketLinkCaches,
} from "../model/ticketLinks";
import type { LinkedWorkItem, TicketRef } from "../model/session";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

const STALE_AT = Date.now() - 10 * 60_000;
const TICKET: TicketRef = {
  tracker: "jira",
  key: "ENG-42",
  url: "https://acme.atlassian.net/browse/ENG-42",
  title: "Login fails on Safari when the password manager autofills",
  status: "In Progress",
  statusGroup: "started",
  fetchedAt: STALE_AT,
};
const FILLED: TicketRef = {
  ...TICKET,
  status: "Done",
  statusGroup: "completed",
};

let container: HTMLDivElement;
let root: Root;
let connected: string[];
let fetchTicket: (ticket: TicketRef) => Promise<TicketRef>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  resetTicketLinkCaches();
  connected = ["jira", "linear"];
  fetchTicket = async () => FILLED;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.mocked(openUrl).mockReset().mockResolvedValue(undefined);
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "ticket_connected") return connected;
    if (command === "ticket_fetch")
      return fetchTicket((args as { ticket: TicketRef }).ticket);
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

type Props = {
  ticket?: TicketRef;
  visible?: boolean;
  onChange?: (item: LinkedWorkItem | undefined) => void;
  current?: () => LinkedWorkItem | undefined;
};

async function render(props: Props = {}) {
  const ticket = props.ticket ?? TICKET;
  await act(async () =>
    root.render(
      createElement(SessionTicketChip, {
        ticket,
        visible: props.visible ?? true,
        onChange: props.onChange ?? (() => undefined),
        current: props.current ?? (() => ({ kind: "ticket" as const, ticket })),
      }),
    ),
  );
}

function fetches() {
  return vi.mocked(invoke).mock.calls.filter(([c]) => c === "ticket_fetch");
}

async function openMenu() {
  await act(async () =>
    container
      .querySelector<HTMLButtonElement>('[aria-label="Ticket actions"]')!
      .click(),
  );
}

async function pick(label: string) {
  const item = [
    ...container.querySelectorAll<HTMLElement>('[role="menuitem"]'),
  ].find((el) => el.textContent === label);
  expect(item, label).toBeTruthy();
  await act(async () => item!.click());
}

it("shows tracker, key, title and a status pill coloured by group", async () => {
  await render({ visible: false });
  const text = container.textContent!;
  expect(text).toContain("Jira");
  expect(text).toContain("ENG-42");
  expect(text).toContain("Login fails on Safari");
  const pill = container.querySelector('[data-status-group="started"]')!;
  expect(pill.textContent).toBe("In Progress");
  expect(container.querySelector('[data-stale="true"]')).toBeNull();
});

it("shows only the key when nothing has been fetched yet", async () => {
  fetchTicket = () => new Promise(() => undefined);
  await render({
    ticket: {
      tracker: "linear",
      key: "ENG-7",
      url: null,
      title: null,
      status: null,
      statusGroup: null,
      fetchedAt: null,
    },
  });
  expect(container.textContent).toContain("ENG-7");
  expect(container.querySelector("[data-status-group]")).toBeNull();
});

it("marks a ticket stale when its tracker is no longer connected", async () => {
  connected = ["linear"];
  await render({ visible: false });
  const stale = container.querySelector('[data-stale="true"]')!;
  expect(stale.getAttribute("title")).toBe(
    "Jira is disconnected. Reconnect in Settings.",
  );
});

it("loads the connected set once, and again only when connections change", async () => {
  await render({ visible: false });
  await render({ visible: false });
  const calls = () =>
    vi.mocked(invoke).mock.calls.filter(([c]) => c === "ticket_connected")
      .length;
  expect(calls()).toBe(1);
  connected = [];
  await act(async () => notifyTicketConnectionsChanged());
  expect(calls()).toBe(2);
  expect(container.querySelector('[data-stale="true"]')).not.toBeNull();
});

it("refreshes once when the session is opened and saves the filled ticket", async () => {
  const onChange = vi.fn();
  await render({ onChange });
  expect(fetches()).toHaveLength(1);
  expect(onChange).toHaveBeenCalledWith({ kind: "ticket", ticket: FILLED });
  await render({ onChange });
  expect(fetches()).toHaveLength(1);
});

it("skips the open-time refresh when the snapshot is under a minute old", async () => {
  await render({ ticket: { ...TICKET, fetchedAt: Date.now() - 5_000 } });
  expect(fetches()).toHaveLength(0);
});

it("explicit Refresh runs even on a fresh snapshot", async () => {
  await render({
    visible: false,
    ticket: { ...TICKET, fetchedAt: Date.now() },
  });
  await openMenu();
  await pick("Refresh");
  expect(fetches()).toHaveLength(1);
});

it("does not refresh a session that is not open", async () => {
  await render({ visible: false });
  expect(fetches()).toHaveLength(0);
});

it("drops a fetch that lands after the session link changed", async () => {
  let release!: (ticket: TicketRef) => void;
  fetchTicket = () => new Promise((resolve) => (release = resolve));
  const onChange = vi.fn();
  let linked: LinkedWorkItem | undefined = { kind: "ticket", ticket: TICKET };
  await render({ onChange, current: () => linked });
  linked = {
    kind: "ticket",
    ticket: { ...TICKET, tracker: "linear", key: "ENG-9" },
  };
  await act(async () => release(FILLED));
  expect(onChange).not.toHaveBeenCalled();
});

it("keeps the cached ticket and explains when a refresh fails", async () => {
  fetchTicket = async () => {
    throw "Jira is unreachable";
  };
  const onChange = vi.fn();
  await render({ onChange });
  expect(onChange).not.toHaveBeenCalled();
  expect(container.textContent).toContain("ENG-42");
  expect(container.querySelector('[role="status"]')?.textContent).toBe(
    "Jira is unreachable",
  );
});

it("opens the ticket in its tracker from the menu", async () => {
  await render({ visible: false });
  await openMenu();
  await pick("Open in Jira");
  expect(openUrl).toHaveBeenCalledWith(TICKET.url);
});

it("refreshes on demand from the menu", async () => {
  const onChange = vi.fn();
  await render({ visible: false, onChange });
  await openMenu();
  await pick("Refresh");
  expect(fetches()).toHaveLength(1);
  expect(onChange).toHaveBeenCalledWith({ kind: "ticket", ticket: FILLED });
});

it("unlinks from the menu", async () => {
  const onChange = vi.fn();
  await render({ visible: false, onChange });
  await openMenu();
  await pick("Unlink");
  expect(onChange).toHaveBeenCalledWith(undefined);
});
