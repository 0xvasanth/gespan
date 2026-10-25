// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { resetTicketLinkCaches } from "../../sessions/model/ticketLinks";
import type { TicketRef } from "../../sessions/model/session";
import type { BoardCard } from "../model/board";
import { CardDetail } from "./CardDetail";

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
    columnName: "In Progress",
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

const dialogText = () => document.querySelector('[role="dialog"]')!.textContent;

describe("CardDetail activity", () => {
  it("says there is no activity only when the load succeeded empty", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    const props = await renderDetail();
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
    expect(dialogText()).not.toContain("No activity yet");
    expect(dialogText()).toContain("Couldn't load activity");
  });
});

describe("CardDetail linked work", () => {
  const openButton = () =>
    [
      ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
    ].find((button) => button.textContent?.includes("Open linked work"));

  it("says a linked session missing from the snapshot no longer exists", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await renderDetail({
      card: { ...CARD, linkKind: "session", linkId: "gone" },
      link: { kind: "session", target: null },
      linkTargets: [{ kind: "session", id: "s1", label: "Other session" }],
    });
    const chip = document.querySelector('[role="dialog"] [data-link-chip]')!;
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
      '[role="dialog"] button[aria-label^="Linked work"]',
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
        .querySelector('[role="dialog"] button[aria-label^="Linked work"]')!
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
      ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
    ].find((button) => button.textContent?.includes("Delete card"))!;

  const deletes = () =>
    vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "board_card_delete");

  async function settle() {
    await act(async () => {
      for (let i = 0; i < 5; i += 1) await Promise.resolve();
    });
  }

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

  const settle = () =>
    act(async () => {
      for (let i = 0; i < 10; i += 1) await Promise.resolve();
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
});
