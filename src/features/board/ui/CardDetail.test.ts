// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
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
    const elsewhere = { kind: "session" as const, id: "s9", label: "Elsewhere" };
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
