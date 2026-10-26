// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { LinkedWorkItem, TicketRef } from "../model/session";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../files/model/fileIndex", () => ({
  loadProjectFiles: vi.fn(() => new Promise(() => {})),
  peekProjectFiles: vi.fn(() => []),
  rankProjectFiles: vi.fn(() => []),
  recentOpenedFiles: vi.fn(() => []),
  rememberOpenedFile: vi.fn(),
}));
vi.mock("../../projects/model/recents", () => ({
  looksLikeProject: (path: string) => path !== "~",
}));

const { FilePicker } = await import("../../files/ui/FilePicker");
const { setSessionLinkedWorkItem } = await import("../data/sessionStore");
const { resetTicketLinkCaches } = await import("../model/ticketLinks");
const { NEW_SESSION_FROM_TICKET_ACTION } = await import(
  "../model/ticketSession"
);
const { NewSessionFromTicketDialog } = await import(
  "./NewSessionFromTicketDialog"
);
type TicketSessionHost = import("../model/ticketSession").TicketSessionHost;

const LINEAR: TicketRef = {
  tracker: "linear",
  key: "ENG-7",
  url: "https://linear.app/acme/issue/ENG-7",
  title: null,
  status: null,
  statusGroup: null,
  fetchedAt: null,
};

let container: HTMLDivElement;
let root: Root;
let offline: boolean;

/** Sessions as the app holds them: id → link and composer draft. */
type FakeSession = { linked?: LinkedWorkItem; draft: string; cwd: string };
let sessions: Map<string, FakeSession>;
let settled: Promise<string> | null;

/** The app's side of the flow, with the link going through the real store. */
function fakeHost(): TicketSessionHost {
  return {
    createSession(draft) {
      const id = `session-${sessions.size + 1}`;
      sessions.set(id, { draft, cwd: "/repo" });
      return id;
    },
    link(id, item) {
      sessions.get(id)!.linked = item;
      void setSessionLinkedWorkItem(id, item);
    },
    linkedWorkItem: (id) => sessions.get(id)?.linked,
    draft: (id) => sessions.get(id)?.draft,
    setDraft(id, text) {
      sessions.get(id)!.draft = text;
    },
  };
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  sessions = new Map();
  settled = null;
  offline = false;
  resetTicketLinkCaches();
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    const input = (args as { input?: string } | undefined)?.input ?? "";
    switch (command) {
      case "ticket_resolve":
        // The Rust resolver: a Linear key or link resolves; anything else,
        // a GitHub URL included, is refused.
        return /^ENG-\d+$/.test(input) || input.startsWith("https://linear.app/")
          ? { kind: "one", ticket: LINEAR }
          : { kind: "error", message: "Not a ticket key or link" };
      case "ticket_fetch":
        if (offline) throw "Linear is unreachable";
        return {
          ...(args as { ticket: TicketRef }).ticket,
          title: "Fix login",
          status: "Todo",
          statusGroup: "unstarted",
          fetchedAt: 1_700_000_000_000,
        };
      case "session_set_linked_work_item":
        return undefined;
      default:
        throw new Error(`Unexpected command: ${command}`);
    }
  });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function calls(command: string) {
  return vi.mocked(invoke).mock.calls.filter(([c]) => c === command);
}

async function openDialog() {
  const onClose = vi.fn();
  await act(async () =>
    root.render(
      createElement(NewSessionFromTicketDialog, {
        host: fakeHost(),
        description: "repo",
        onStarted: (pending: Promise<string>) => {
          settled = pending;
        },
        onClose,
      }),
    ),
  );
  return { onClose };
}

async function type(value: string) {
  const field = document.querySelector<HTMLInputElement>("form input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function submit() {
  await act(async () => {
    document
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}

it("palette_new_session_from_ticket_links_and_drafts_title", async () => {
  // The palette lists the command and runs it by id.
  const onRunAction = vi.fn();
  await act(async () =>
    root.render(
      createElement(FilePicker, {
        open: true,
        cwd: "/repo",
        initialQuery: ">new session from",
        onOpenFile: vi.fn(),
        onRunAction,
        extraActions: [NEW_SESSION_FROM_TICKET_ACTION],
        onClose: vi.fn(),
      }),
    ),
  );
  const search = document.querySelector<HTMLInputElement>(
    "[data-file-picker] input",
  )!;
  expect(document.querySelector("[data-file-picker]")!.textContent).toContain(
    "New session from ticket…",
  );
  await act(async () => {
    search.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
  });
  expect(onRunAction).toHaveBeenCalledWith("session-from-ticket");

  const { onClose } = await openDialog();
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain(
    "New session from ticket",
  );
  await type("ENG-7");
  await submit();
  expect(onClose).toHaveBeenCalled();
  const id = await settled!;

  expect([...sessions.keys()]).toEqual([id]);
  const session = sessions.get(id)!;
  expect(session.linked?.kind).toBe("ticket");
  const ticket = session.linked?.kind === "ticket" ? session.linked.ticket : null;
  expect([ticket?.tracker, ticket?.key, ticket?.title]).toEqual([
    "linear",
    "ENG-7",
    "Fix login",
  ]);
  expect(session.draft).toBe("Fix login");
  expect(calls("ticket_fetch")).toHaveLength(1);
  // The link is saved through the session store, first as the key, then
  // filled in by the fetch.
  const saved = calls("session_set_linked_work_item").map(
    ([, args]) => args as { sessionId: string; linkedWorkItem: LinkedWorkItem },
  );
  expect(saved.map((a) => a.sessionId)).toEqual([id, id]);
  expect(saved[0].linkedWorkItem).toEqual({ kind: "ticket", ticket: LINEAR });
  expect(saved[1].linkedWorkItem).toMatchObject({
    kind: "ticket",
    ticket: { key: "ENG-7", title: "Fix login" },
  });
  // Nothing is sent: the only commands are resolve, fetch and the link.
  expect(
    new Set(vi.mocked(invoke).mock.calls.map(([c]) => c)),
  ).toEqual(
    new Set(["ticket_resolve", "ticket_fetch", "session_set_linked_work_item"]),
  );
});

it("new_session_from_ticket_keeps_the_key_draft_when_the_fetch_fails", async () => {
  offline = true;
  await openDialog();
  await type("ENG-7");
  await submit();
  const id = await settled!;

  const session = sessions.get(id)!;
  expect(session.linked).toEqual({ kind: "ticket", ticket: LINEAR });
  expect(session.draft).toBe("ENG-7");
  expect(calls("ticket_fetch")).toHaveLength(1);
});

it("keeps a draft the user edited while the ticket was loading", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  const real = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "ticket_fetch") await gate;
    return real(command, args);
  });
  await openDialog();
  await type("ENG-7");
  await submit();
  const [id] = sessions.keys();
  expect(sessions.get(id)!.draft).toBe("ENG-7");
  sessions.get(id)!.draft = "ENG-7 look at the redirect";
  release();
  await settled;

  expect(sessions.get(id)!.draft).toBe("ENG-7 look at the redirect");
  const linked = sessions.get(id)!.linked;
  expect(linked?.kind === "ticket" && linked.ticket.title).toBe("Fix login");
});

it("new_session_from_ticket_refuses_a_github_url_and_creates_nothing", async () => {
  const { onClose } = await openDialog();
  await type("https://github.com/acme/app/issues/5");
  await submit();

  expect(calls("ticket_resolve")[0][1]).toEqual({
    input: "https://github.com/acme/app/issues/5",
  });
  expect(document.querySelector('[role="alert"]')?.textContent).toBe(
    "Not a ticket key or link",
  );
  expect(sessions.size).toBe(0);
  expect(settled).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
  expect(calls("session_set_linked_work_item")).toHaveLength(0);
  expect(calls("ticket_fetch")).toHaveLength(0);
});
