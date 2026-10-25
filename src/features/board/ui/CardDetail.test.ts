// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { resetTicketLinkCaches } from "../../sessions/model/ticketLinks";
import type { TicketRef } from "../../sessions/model/session";
import type { BoardCard, BoardLabel } from "../model/board";
import { CardDetail } from "./CardDetail";
import { LABEL_COLORS } from "./boardFormat";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));

const CARD: BoardCard = {
  id: "c3",
  boardId: "b1",
  columnId: "progress",
  number: 3,
  title: "Ship board",
  body: "",
  position: 0,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: 1,
};

const COLUMNS = [
  { id: "backlog", name: "Backlog", cardCount: 2 },
  { id: "progress", name: "In Progress", cardCount: 1 },
  { id: "done", name: "Done", cardCount: 0 },
];

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(invoke).mockReset();
  vi.mocked(ask).mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

async function renderDetail(
  overrides: Partial<ComponentProps<typeof CardDetail>> = {},
) {
  const props: ComponentProps<typeof CardDetail> = {
    card: CARD,
    columns: COLUMNS,
    labels: [],
    projects: [],
    link: null,
    linkTargets: [],
    onClose: vi.fn(),
    onOpenLinkedWork: vi.fn(),
    onError: vi.fn(),
    ...overrides,
  };
  await act(async () => root.render(createElement(CardDetail, props)));
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await Promise.resolve();
  });
  return props;
}

const dialogText = () =>
  document.querySelector("[data-card-drawer]")!.textContent;

async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

const tab = (name: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(
    (el) => el.textContent === name,
  )!;

async function openTab(name: string) {
  await act(async () => tab(name).click());
  await settle();
}

describe("CardDetail activity", () => {
  it("says there is no activity only when the load succeeded empty", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    const props = await renderDetail();
    await openTab("Activity");
    expect(dialogText()).toContain("No activity yet");
    expect(props.onError).not.toHaveBeenCalled();
  });

  it("reports a failed activity load instead of showing an empty history", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd) => {
      if (cmd === "board_activity") throw "database is locked";
      return undefined;
    });
    const props = await renderDetail();
    expect(props.onError).toHaveBeenCalledTimes(1);
    expect(props.onError).toHaveBeenCalledWith(
      "Couldn't load activity for card #3: database is locked",
    );
    await openTab("Activity");
    expect(dialogText()).not.toContain("No activity yet");
    expect(dialogText()).toContain("Couldn't load activity");
  });
});

describe("CardDetail linked work", () => {
  const openButton = () =>
    [
      ...document.querySelectorAll<HTMLButtonElement>(
        "[data-card-drawer] button",
      ),
    ].find((button) => button.textContent?.includes("Open linked work"));

  it("says a linked session missing from the snapshot no longer exists", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await renderDetail({
      card: { ...CARD, linkKind: "session", linkId: "gone" },
      link: { kind: "session", target: null },
      linkTargets: [{ kind: "session", id: "s1", label: "Other session" }],
    });
    const chip = document.querySelector("[data-card-drawer] [data-link-chip]")!;
    expect(chip.getAttribute("data-link-chip")).toBe("missing");
    expect(chip.textContent).toBe("Linked work no longer exists");
    expect(openButton()).toBeUndefined();
  });

  it("leaves archived sessions out of the picker but still shows the card's own", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    const archived = {
      kind: "session" as const,
      id: "s-old",
      label: "Old work",
      unlisted: true,
    };
    const otherArchived = { ...archived, id: "s-older", label: "Older work" };
    const active = { kind: "session" as const, id: "s1", label: "Fix login" };
    await renderDetail({
      card: { ...CARD, linkKind: "session", linkId: "s-old" },
      link: { kind: "session", target: archived },
      linkTargets: [active, archived, otherArchived],
    });
    const trigger = document.querySelector<HTMLButtonElement>(
      '[data-card-drawer] button[aria-label^="Linked work"]',
    )!;
    expect(trigger.getAttribute("aria-label")).toBe(
      "Linked work: Session · Old work",
    );
    act(() => trigger.click());
    const options = [...document.querySelectorAll('[role="option"]')].map(
      (el) => el.textContent,
    );
    expect(options).toEqual([
      "No linked work",
      "Session · Fix login",
      "Session · Old work",
    ]);
    expect(openButton()).toBeDefined();
  });

  it("shows a linked target the picker list doesn't hold", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    const elsewhere = {
      kind: "session" as const,
      id: "s9",
      label: "Elsewhere",
    };
    await renderDetail({
      card: { ...CARD, linkKind: "session", linkId: "s9" },
      link: { kind: "session", target: elsewhere },
      linkTargets: [],
    });
    expect(
      document
        .querySelector('[data-card-drawer] button[aria-label^="Linked work"]')!
        .getAttribute("aria-label"),
    ).toBe("Linked work: Session · Elsewhere");
  });

  it("offers to open linked work that still exists", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    const target = { kind: "session" as const, id: "s1", label: "Fix login" };
    const props = await renderDetail({
      card: { ...CARD, linkKind: "session", linkId: "s1" },
      link: { kind: "session", target },
      linkTargets: [target],
    });
    expect(dialogText()).not.toContain("Linked work no longer exists");
    await act(async () => openButton()!.click());
    expect(props.onOpenLinkedWork).toHaveBeenCalledWith(target);
  });
});

describe("CardDetail delete", () => {
  const deleteButton = () =>
    [
      ...document.querySelectorAll<HTMLButtonElement>(
        "[data-card-drawer] button",
      ),
    ].find((button) => button.textContent?.includes("Delete card"))!;

  const deletes = () =>
    vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "board_card_delete");

  it("asks before deleting and keeps the card when declined", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    vi.mocked(ask).mockResolvedValue(false);
    const props = await renderDetail();
    await act(async () => deleteButton().click());
    await settle();
    expect(ask).toHaveBeenCalledTimes(1);
    expect(vi.mocked(ask).mock.calls[0][0]).toContain("#3");
    expect(vi.mocked(ask).mock.calls[0][0]).toContain("Ship board");
    expect(deletes()).toHaveLength(0);
    expect(props.onClose).not.toHaveBeenCalled();
  });

  it("deletes the card and closes once confirmed", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd) =>
      cmd === "board_activity" ? [] : undefined,
    );
    vi.mocked(ask).mockResolvedValue(true);
    const props = await renderDetail();
    await act(async () => deleteButton().click());
    await settle();
    expect(deletes()).toEqual([["board_card_delete", { cardId: "c3" }]]);
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("does not delete when the confirm dialog can't open", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    vi.mocked(ask).mockRejectedValue(new Error("dialog unavailable"));
    const props = await renderDetail();
    await act(async () => deleteButton().click());
    await settle();
    expect(deletes()).toHaveLength(0);
    expect(props.onClose).not.toHaveBeenCalled();
  });
});

describe("CardDetail ticket link", () => {
  const LINEAR: TicketRef = {
    tracker: "linear",
    key: "ENG-7",
    url: "https://linear.app/acme/issue/ENG-7",
    title: null,
    status: null,
    statusGroup: null,
    fetchedAt: null,
  };
  const FETCHED: TicketRef = {
    ...LINEAR,
    title: "Fix login",
    status: "In Progress",
    statusGroup: "started",
    fetchedAt: Date.now() - 10 * 60_000,
  };

  const writes = () =>
    vi
      .mocked(invoke)
      .mock.calls.filter(([cmd]) => cmd === "board_card_set_ticket");
  const fetches = () =>
    vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "ticket_fetch");

  beforeEach(() => {
    resetTicketLinkCaches();
    vi.mocked(invoke).mockImplementation(async (cmd, args) => {
      if (cmd === "board_activity") return [];
      if (cmd === "ticket_connected") return ["linear"];
      if (cmd === "board_card_ticket") return FETCHED;
      if (cmd === "ticket_resolve") {
        const input = (args as { input: string }).input;
        return input.includes("github.com")
          ? { kind: "error", message: "Not a ticket key or link" }
          : { kind: "one", ticket: LINEAR };
      }
      if (cmd === "ticket_fetch")
        return { ...(args as { ticket: TicketRef }).ticket, ...FETCHED };
      return undefined;
    });
  });

  const buttonByText = (text: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (button) => button.textContent === text,
    );

  async function typeAndSubmit(value: string) {
    const field = document.querySelector<HTMLInputElement>(
      'input[aria-label^="Ticket link"]',
    )!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(field, value);
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      field
        .closest("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        );
    });
    await settle();
  }

  it("links a ticket from the card and saves it on the card", async () => {
    await renderDetail();
    expect(dialogText()).toContain("Ticket");
    await act(async () => buttonByText("Link ticket…")!.click());
    // A card takes tickets only.
    expect(document.body.textContent).toContain(
      "Paste a Jira, Linear or Plane link",
    );
    await typeAndSubmit("ENG-7");
    expect(writes()).toEqual([
      ["board_card_set_ticket", { cardId: "c3", ticket: LINEAR }],
    ]);
  });

  it("refuses a GitHub link instead of saving it as the card's link", async () => {
    await renderDetail();
    await act(async () => buttonByText("Link ticket…")!.click());
    await typeAndSubmit("https://github.com/acme/app/pull/12");
    expect(writes()).toHaveLength(0);
    expect(
      vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "board_card_link"),
    ).toHaveLength(0);
    expect(document.body.textContent).toContain("Not a ticket key or link");
  });

  it("shows the chip and reads the ticket when the card opens", async () => {
    await renderDetail({ ticket: FETCHED });
    const chip = document.querySelector("[data-card-ticket]")!;
    expect(chip.textContent).toContain("ENG-7");
    expect(chip.textContent).toContain("Fix login");
    expect(chip.textContent).toContain("Change ticket…");
    await settle();
    expect(fetches()).toHaveLength(1);
    expect(writes()).toHaveLength(1);
    expect(writes()[0][1]).toMatchObject({ cardId: "c3" });
  });

  it("does not read a ticket that was read less than a minute ago", async () => {
    await renderDetail({
      ticket: { ...FETCHED, fetchedAt: Date.now() - 5_000 },
    });
    await settle();
    expect(fetches()).toHaveLength(0);
    expect(writes()).toHaveLength(0);
  });

  it("unlinks through the chip menu", async () => {
    await renderDetail({ ticket: { ...FETCHED, fetchedAt: Date.now() } });
    await act(async () =>
      document
        .querySelector<HTMLButtonElement>('[aria-label="Ticket actions"]')!
        .click(),
    );
    const unlink = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ].find((el) => el.textContent === "Unlink")!;
    await act(async () => unlink.click());
    expect(writes()).toEqual([
      ["board_card_set_ticket", { cardId: "c3", ticket: null }],
    ]);
  });

  it("reports a failed save", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd) => {
      if (cmd === "board_activity") return [];
      if (cmd === "ticket_resolve") return { kind: "one", ticket: LINEAR };
      if (cmd === "board_card_set_ticket") throw "database is locked";
      return undefined;
    });
    const props = await renderDetail();
    await act(async () => buttonByText("Link ticket…")!.click());
    await typeAndSubmit("ENG-7");
    expect(props.onError).toHaveBeenCalledWith(
      "Couldn't link a ticket to card #3: database is locked",
    );
  });

  describe("a fetch that resolves late", () => {
    let release: () => void;
    let current: TicketRef | null;

    beforeEach(() => {
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      current = FETCHED;
      const base = vi.mocked(invoke).getMockImplementation()!;
      vi.mocked(invoke).mockImplementation(async (cmd, args) => {
        if (cmd === "board_card_ticket") return current;
        if (cmd === "ticket_fetch") await gate;
        return base(cmd, args);
      });
    });

    it("does not re-link a ticket the user unlinked meanwhile", async () => {
      await renderDetail({ ticket: FETCHED });
      expect(fetches()).toHaveLength(1);
      current = null;
      release();
      await settle();
      expect(writes()).toHaveLength(0);
    });

    it("keeps a ticket the user relinked to another key meanwhile", async () => {
      await renderDetail({ ticket: FETCHED });
      current = { ...FETCHED, key: "ENG-8" };
      release();
      await settle();
      expect(writes()).toHaveLength(0);
    });

    it("saves when the card still links the ticket", async () => {
      await renderDetail({ ticket: FETCHED });
      release();
      await settle();
      expect(writes()).toHaveLength(1);
    });
  });

  it("retitles a card still titled with the key once a refresh succeeds", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (cmd, args) => {
      if (cmd === "board_card_ticket") return FETCHED;
      if (cmd === "board_get")
        return {
          board: { id: "b1" },
          columns: [],
          cards: [{ ...CARD, title: "ENG-7", body: "notes" }],
        };
      return base(cmd, args);
    });
    await renderDetail({ card: { ...CARD, title: "ENG-7" }, ticket: FETCHED });
    await settle();
    const updates = vi
      .mocked(invoke)
      .mock.calls.filter(([cmd]) => cmd === "board_card_update");
    expect(updates).toEqual([
      [
        "board_card_update",
        { cardId: "c3", title: "Fix login", body: "notes" },
      ],
    ]);
  });

  it("leaves a title the user changed when a refresh succeeds", async () => {
    const base = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (cmd, args) => {
      if (cmd === "board_card_ticket") return FETCHED;
      if (cmd === "board_get")
        return { board: { id: "b1" }, columns: [], cards: [CARD] };
      return base(cmd, args);
    });
    await renderDetail({ ticket: FETCHED });
    await settle();
    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter(([cmd]) => cmd === "board_card_update"),
    ).toHaveLength(0);
  });
});

// ---- the docked drawer ----

const drawer = () => document.querySelector<HTMLElement>("[data-card-drawer]")!;
const calls = (cmd: string) =>
  vi.mocked(invoke).mock.calls.filter(([name]) => name === cmd);

function typeInto(
  field: HTMLInputElement | HTMLTextAreaElement,
  value: string,
) {
  const proto =
    field instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  return act(async () => {
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const pressKey = (el: Element, key: string) =>
  act(async () => {
    el.dispatchEvent(
      new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
    );
  });

const field = (label: string) =>
  drawer().querySelector<HTMLInputElement & HTMLTextAreaElement>(
    `[aria-label="${label}"]`,
  )!;

async function openMenu(label: string) {
  await act(async () =>
    drawer()
      .querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
      .click(),
  );
}

const menuItem = (text: string) =>
  [
    ...document.querySelectorAll<HTMLElement>(
      '[role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"]',
    ),
  ].find((el) => el.textContent?.includes(text))!;

async function choose(label: string, text: string) {
  await openMenu(label);
  await act(async () => menuItem(text).click());
  await settle();
}

const label = (id: string, name: string, color = "e5534b"): BoardLabel => ({
  id,
  boardId: "b1",
  name,
  color,
  position: 0,
});

const extras = (over: Partial<import("../model/board").CardExtras> = {}) => ({
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
  ...over,
});

describe("CardDetail drawer", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockImplementation(async (cmd) =>
      cmd === "board_activity" ||
      cmd === "board_subtasks" ||
      cmd === "board_comments"
        ? []
        : undefined,
    );
  });

  it("drawer_is_docked_not_a_modal", async () => {
    const onBoardClick = vi.fn();
    const board = document.createElement("button");
    board.textContent = "board card";
    board.addEventListener("click", onBoardClick);
    document.body.append(board);
    const props = await renderDetail();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(drawer().getAttribute("aria-label")).toBe("Card #3");
    // The board behind it stays reachable.
    board.click();
    expect(onBoardClick).toHaveBeenCalledTimes(1);
    expect(props.onClose).not.toHaveBeenCalled();
    await pressKey(drawer().querySelector("input")!, "Escape");
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("lists the properties in order", async () => {
    await renderDetail();
    const rows = [...drawer().querySelectorAll("[data-property]")].map((el) =>
      el.getAttribute("data-property"),
    );
    expect(rows).toEqual([
      "status",
      "priority",
      "due",
      "projects",
      "labels",
      "linked",
      "ticket",
    ]);
    expect(
      drawer().querySelector('[data-property="status"]')!.textContent,
    ).toContain("In Progress");
  });

  describe("priority_due_projects_labels_save_through_their_commands", () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(new Date(2026, 9, 25, 15, 30));
    });
    afterEach(() => vi.useRealTimers());

    it("moves the card to the chosen column, at the end", async () => {
      await renderDetail();
      await choose("Status", "Backlog");
      expect(calls("board_card_move")).toEqual([
        [
          "board_card_move",
          { cardId: "c3", columnId: "backlog", index: 2, actor: "user" },
        ],
      ]);
      // Choosing the column it is already in writes nothing.
      await choose("Status", "In Progress");
      expect(calls("board_card_move")).toHaveLength(1);
    });

    it("sets the priority by its index", async () => {
      await renderDetail({ extras: extras({ priority: 1 }) });
      expect(
        drawer().querySelector('[data-property="priority"]')!.textContent,
      ).toContain("Low");
      await choose("Priority", "High");
      expect(calls("board_card_set_priority")).toEqual([
        ["board_card_set_priority", { cardId: "c3", priority: 3 }],
      ]);
    });

    it("sets due dates at local midnight and clears them", async () => {
      await renderDetail({ extras: extras({ dueAt: 1 }) });
      await choose("Due", "Today");
      await choose("Due", "Tomorrow");
      await choose("Due", "Next week");
      await choose("Due", "Clear");
      expect(calls("board_card_set_due").map(([, args]) => args)).toEqual([
        { cardId: "c3", dueAt: new Date(2026, 9, 25).getTime() },
        { cardId: "c3", dueAt: new Date(2026, 9, 26).getTime() },
        { cardId: "c3", dueAt: new Date(2026, 10, 1).getTime() },
        { cardId: "c3", dueAt: null },
      ]);
    });

    it("sets a picked date, and clears an emptied date input", async () => {
      await renderDetail({
        extras: extras({ dueAt: new Date(2026, 10, 2).getTime() }),
      });
      const input = field("Due date");
      expect(input.value).toBe("2026-11-02");
      await typeInto(input, "2026-12-31");
      expect(calls("board_card_set_due")).toEqual([
        [
          "board_card_set_due",
          { cardId: "c3", dueAt: new Date(2026, 11, 31).getTime() },
        ],
      ]);
      await typeInto(input, "");
      expect(calls("board_card_set_due")[1][1]).toEqual({
        cardId: "c3",
        dueAt: null,
      });
    });

    it("adds and removes projects, keeping the others", async () => {
      await renderDetail({
        projects: [
          { path: "/a", name: "App" },
          { path: "/b", name: "Blog" },
        ],
        extras: extras({ projects: ["/a"] }),
      });
      await openMenu("Projects");
      expect(menuItem("App").getAttribute("aria-checked")).toBe("true");
      expect(menuItem("Blog").getAttribute("aria-checked")).toBe("false");
      await act(async () => menuItem("Blog").click());
      await act(async () => menuItem("App").click());
      expect(calls("board_card_set_projects").map(([, a]) => a)).toEqual([
        { cardId: "c3", projects: ["/a", "/b"] },
        { cardId: "c3", projects: ["/b"] },
      ]);
    });

    it("keeps a project the workspace no longer lists when toggling others", async () => {
      await renderDetail({
        projects: [{ path: "/b", name: "Blog" }],
        extras: extras({ projects: ["/gone"] }),
      });
      await choose("Projects", "Blog");
      expect(calls("board_card_set_projects")[0][1]).toEqual({
        cardId: "c3",
        projects: ["/gone", "/b"],
      });
    });

    it("assigns and unassigns labels", async () => {
      await renderDetail({
        labels: [label("l1", "Bug"), label("l2", "Docs")],
        extras: extras({ labels: ["l1"] }),
      });
      await openMenu("Labels");
      await act(async () => menuItem("Docs").click());
      await act(async () => menuItem("Bug").click());
      expect(calls("board_card_set_labels").map(([, a]) => a)).toEqual([
        { cardId: "c3", labelIds: ["l1", "l2"] },
        { cardId: "c3", labelIds: ["l2"] },
      ]);
    });

    it("reports a failed priority write", async () => {
      vi.mocked(invoke).mockImplementation(async (cmd) => {
        if (cmd === "board_card_set_priority") throw "database is locked";
        return [];
      });
      const props = await renderDetail();
      await choose("Priority", "Urgent");
      expect(props.onError).toHaveBeenCalledWith(
        "Couldn't set the priority of card #3: database is locked",
      );
    });
  });

  it("create_label_inline_then_assign", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd) => {
      if (cmd === "board_label_create")
        return label("l9", "Urgent bug", "e0823d");
      return cmd === "board_activity" ||
        cmd === "board_subtasks" ||
        cmd === "board_comments"
        ? []
        : undefined;
    });
    await renderDetail({
      labels: [label("l1", "Bug"), label("l2", "Docs")],
      extras: extras({ labels: ["l1"] }),
    });
    await openMenu("Labels");
    await act(async () => menuItem("Create label").click());
    const input = document.querySelector<HTMLInputElement>(
      'input[aria-label="Label name"]',
    )!;
    await typeInto(input, "Urgent bug");
    await pressKey(input, "Enter");
    await settle();
    // The third label takes the third palette colour.
    expect(calls("board_label_create")).toEqual([
      [
        "board_label_create",
        { boardId: "b1", name: "Urgent bug", color: LABEL_COLORS[2] },
      ],
    ]);
    expect(calls("board_card_set_labels")).toEqual([
      ["board_card_set_labels", { cardId: "c3", labelIds: ["l1", "l9"] }],
    ]);
  });

  it("does not create a label with a blank name", async () => {
    await renderDetail();
    await openMenu("Labels");
    await act(async () => menuItem("Create label").click());
    const input = document.querySelector<HTMLInputElement>(
      'input[aria-label="Label name"]',
    )!;
    await typeInto(input, "   ");
    await pressKey(input, "Enter");
    await settle();
    expect(calls("board_label_create")).toHaveLength(0);
  });

  describe("subtasks_add_toggle_delete_update_progress", () => {
    type Sub = {
      id: string;
      cardId: string;
      title: string;
      done: boolean;
      position: number;
    };
    let subtasks: Sub[];
    beforeEach(() => {
      subtasks = [
        { id: "s1", cardId: "c3", title: "Write", done: false, position: 0 },
        { id: "s2", cardId: "c3", title: "Test", done: true, position: 1 },
      ];
      vi.mocked(invoke).mockImplementation(async (cmd, args) => {
        const a = args as Record<string, unknown>;
        if (cmd === "board_subtasks") return subtasks.map((s) => ({ ...s }));
        if (cmd === "board_subtask_add") {
          const sub = {
            id: "s3",
            cardId: "c3",
            title: String(a.title),
            done: false,
            position: 2,
          };
          subtasks.push(sub);
          return sub;
        }
        if (cmd === "board_subtask_toggle") {
          const sub = subtasks.find((s) => s.id === a.subtaskId)!;
          sub.done = Boolean(a.done);
          return undefined;
        }
        if (cmd === "board_subtask_delete") {
          subtasks = subtasks.filter((s) => s.id !== a.subtaskId);
          return undefined;
        }
        return cmd === "board_activity" || cmd === "board_comments"
          ? []
          : undefined;
      });
    });

    const progress = () =>
      drawer().querySelector("[data-subtask-progress]")!.textContent;

    it("adds, checks and deletes, with the progress following", async () => {
      await renderDetail();
      expect(progress()).toBe("1/2");

      const input = field("New subtask");
      await typeInto(input, "  Ship  ");
      await pressKey(input, "Enter");
      await settle();
      expect(calls("board_subtask_add")).toEqual([
        ["board_subtask_add", { cardId: "c3", title: "Ship" }],
      ]);
      expect(progress()).toBe("1/3");
      expect(field("New subtask").value).toBe("");

      await act(async () => field("Done: Write").click());
      await settle();
      expect(calls("board_subtask_toggle")).toEqual([
        ["board_subtask_toggle", { subtaskId: "s1", done: true }],
      ]);
      expect(progress()).toBe("2/3");

      // Unchecking a done subtask reopens it.
      await act(async () => field("Done: Test").click());
      await settle();
      expect(calls("board_subtask_toggle")[1][1]).toEqual({
        subtaskId: "s2",
        done: false,
      });

      await act(async () =>
        drawer()
          .querySelector<HTMLButtonElement>(
            'button[aria-label="Delete subtask Test"]',
          )!
          .click(),
      );
      await settle();
      expect(calls("board_subtask_delete")).toEqual([
        ["board_subtask_delete", { subtaskId: "s2" }],
      ]);
      expect(drawer().textContent).not.toContain("Test");
      expect(progress()).toBe("1/2");
    });

    it("ignores a blank subtask", async () => {
      await renderDetail();
      const input = field("New subtask");
      await typeInto(input, "  ");
      await pressKey(input, "Enter");
      await settle();
      expect(calls("board_subtask_add")).toHaveLength(0);
    });

    it("hides the progress when there are no subtasks", async () => {
      subtasks = [];
      await renderDetail();
      expect(drawer().querySelector("[data-subtask-progress]")).toBeNull();
    });
  });

  describe("comments_tab_adds_a_comment", () => {
    it("lists comments and adds one", async () => {
      let comments = [
        {
          id: "m1",
          cardId: "c3",
          body: "Looks good",
          author: "user",
          createdAt: Date.now() - 3 * 60_000,
          editedAt: null,
        },
      ];
      vi.mocked(invoke).mockImplementation(async (cmd, args) => {
        if (cmd === "board_comments") return comments;
        if (cmd === "board_comment_add") {
          const c = {
            id: "m2",
            cardId: "c3",
            body: String((args as { body: string }).body),
            author: "user",
            createdAt: Date.now(),
            editedAt: null,
          };
          comments = [...comments, c];
          return c;
        }
        return cmd === "board_activity" || cmd === "board_subtasks"
          ? []
          : undefined;
      });
      await renderDetail();
      expect(
        drawer().querySelector('[role="tab"][aria-selected="true"]')!
          .textContent,
      ).toBe("Comments");
      expect(dialogText()).toContain("Looks good");

      const input = field("Add a comment");
      const send = () =>
        drawer().querySelector<HTMLButtonElement>(
          'button[aria-label="Send comment"]',
        )!;
      expect(send().disabled).toBe(true);
      await typeInto(input, " Ship it ");
      await act(async () => send().click());
      await settle();
      expect(calls("board_comment_add")).toEqual([
        ["board_comment_add", { cardId: "c3", body: "Ship it" }],
      ]);
      expect(dialogText()).toContain("Ship it");
      expect(field("Add a comment").value).toBe("");
    });
  });

  it("activity_tab_shows_changed_details", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd) => {
      if (cmd === "board_activity")
        return [
          {
            id: "a1",
            boardId: "b1",
            cardId: "c3",
            kind: "edited",
            actor: "user",
            fromColumn: null,
            toColumn: null,
            detail: "priority: High",
            createdAt: Date.now(),
          },
        ];
      return cmd === "board_subtasks" || cmd === "board_comments"
        ? []
        : undefined;
    });
    await renderDetail();
    await openTab("Activity");
    expect(drawer().querySelector("[data-card-activity]")!.textContent).toBe(
      "Changed priority: High · You · just now",
    );
  });

  describe("description and title drafts", () => {
    it("saves the description on blur, not on every keystroke", async () => {
      await renderDetail();
      const body = field("Description");
      await typeInto(body, "Some notes");
      expect(calls("board_card_update")).toHaveLength(0);
      await act(async () =>
        body.dispatchEvent(new FocusEvent("focusout", { bubbles: true })),
      );
      expect(calls("board_card_update")).toEqual([
        [
          "board_card_update",
          { cardId: "c3", title: "Ship board", body: "Some notes" },
        ],
      ]);
    });

    it("board_changed_event_does_not_overwrite_an_unsaved_title_draft", async () => {
      const props = await renderDetail();
      const title = field("Title");
      await act(async () => title.focus());
      await typeInto(title, "My draft");
      // Native writes the same card: the refresh arrives with a new title.
      await act(async () =>
        root.render(
          createElement(CardDetail, {
            ...props,
            card: { ...CARD, title: "Renamed elsewhere", updatedAt: 2 },
          }),
        ),
      );
      expect(field("Title").value).toBe("My draft");
      expect(calls("board_card_update")).toHaveLength(0);
      await act(async () =>
        field("Title").dispatchEvent(
          new FocusEvent("focusout", { bubbles: true }),
        ),
      );
      expect(calls("board_card_update")).toEqual([
        ["board_card_update", { cardId: "c3", title: "My draft", body: "" }],
      ]);
    });

    it("keeps an unsaved description draft across a refresh", async () => {
      const props = await renderDetail();
      await typeInto(field("Description"), "half-written");
      await act(async () =>
        root.render(
          createElement(CardDetail, {
            ...props,
            card: { ...CARD, body: "remote notes", updatedAt: 2 },
          }),
        ),
      );
      expect(field("Description").value).toBe("half-written");
    });

    it("takes a remote change when nothing is being edited", async () => {
      const props = await renderDetail();
      await act(async () =>
        root.render(
          createElement(CardDetail, {
            ...props,
            card: {
              ...CARD,
              title: "Renamed elsewhere",
              body: "remote",
              updatedAt: 2,
            },
          }),
        ),
      );
      expect(field("Title").value).toBe("Renamed elsewhere");
      expect(field("Description").value).toBe("remote");
    });

    it("saves a pending draft when the drawer closes", async () => {
      const props = await renderDetail();
      await typeInto(field("Title"), "Closing draft");
      await pressKey(field("Title"), "Escape");
      expect(calls("board_card_update")).toEqual([
        [
          "board_card_update",
          { cardId: "c3", title: "Closing draft", body: "" },
        ],
      ]);
      expect(props.onClose).toHaveBeenCalledTimes(1);
    });

    it("puts the old title back when it is emptied", async () => {
      await renderDetail();
      await typeInto(field("Title"), "   ");
      await act(async () =>
        field("Title").dispatchEvent(
          new FocusEvent("focusout", { bubbles: true }),
        ),
      );
      expect(calls("board_card_update")).toHaveLength(0);
      expect(field("Title").value).toBe("Ship board");
    });
  });
});
