// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { resetTicketLinkCaches } from "../../sessions/model/ticketLinks";
import type { TicketRef } from "../../sessions/model/session";
import type { BoardCard, BoardLabel, CardExtras } from "../model/board";
import { BoardCardItem } from "./BoardCardItem";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

const CARD: BoardCard = {
  id: "c1",
  boardId: "b1",
  columnId: "todo",
  number: 1,
  title: "Fix login",
  body: "",
  position: 0,
  linkKind: null,
  linkId: null,
  createdAt: 1,
  updatedAt: 1,
};
const TICKET: TicketRef = {
  tracker: "linear",
  key: "ENG-7",
  url: null,
  title: "Fix login",
  status: "Todo",
  statusGroup: "unstarted",
  fetchedAt: 1,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  resetTicketLinkCaches();
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (cmd) =>
    cmd === "ticket_connected" ? ["linear"] : undefined,
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

const BUG: BoardLabel = {
  id: "l1",
  boardId: "b1",
  name: "Bug",
  color: "e5484d",
  position: 0,
};
const EXTRAS: CardExtras = {
  priority: 0,
  dueAt: null,
  projects: [],
  labels: [],
  subtasksDone: 0,
  subtasksTotal: 0,
  comments: 0,
};

async function render(
  props: Partial<Parameters<typeof BoardCardItem>[0]> = {},
) {
  const onOpen = vi.fn();
  await act(async () =>
    root.render(
      createElement(BoardCardItem, {
        card: CARD,
        done: false,
        link: null,
        ticket: TICKET,
        onSetPriority: vi.fn(),
        onSetDue: vi.fn(),
        onSetLabels: vi.fn(),
        onCreateLabel: vi.fn(),
        ...props,
        onOpen,
        onOpenLinkedWork: vi.fn(),
        onRename: vi.fn(),
        onLink: vi.fn(),
        onDelete: vi.fn(),
      }),
    ),
  );
  return onOpen;
}

it("opens the card when its ticket chip is clicked", async () => {
  const onOpen = await render();
  const chip = container.querySelector("[data-card-ticket] span")!;
  await act(async () => (chip as HTMLElement).click());
  expect(onOpen).toHaveBeenCalledTimes(1);
});

it("does not open the card from the chip's own actions button", async () => {
  const onOpen = await render();
  await act(async () =>
    container
      .querySelector<HTMLButtonElement>('[aria-label="Ticket actions"]')!
      .click(),
  );
  expect(onOpen).not.toHaveBeenCalled();
  expect(container.querySelector('[role="menu"]')).not.toBeNull();
});

/**
 * What a sortable wrapper would see. React handles events at the root, so a
 * stop there is what keeps them from listeners above it.
 */
function watchParent() {
  const seen: string[] = [];
  for (const type of ["pointerdown", "keydown", "click"]) {
    document.body.addEventListener(type, () => seen.push(type));
  }
  return seen;
}

it("quick edits never reach the sortable wrapper or open the card", async () => {
  const onSetPriority = vi.fn();
  const onOpen = await render({
    extras: { ...EXTRAS, priority: 2 },
    labels: [BUG],
    allLabels: [BUG],
    onSetPriority,
  });
  const seen = watchParent();
  const trigger = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Priority of #1"]',
  )!;
  await act(async () => {
    trigger.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    trigger.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    trigger.click();
  });
  // Only the face's own controls matter: the menu is portalled to the body.
  expect(seen).toEqual([]);
  const item = [
    ...document.querySelectorAll<HTMLElement>('[role^="menuitem"]'),
  ].find((el) => el.textContent === "Urgent")!;
  await act(async () => {
    item.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    item.click();
  });
  expect(onSetPriority).toHaveBeenCalledWith(4);
  expect(onOpen).not.toHaveBeenCalled();
  seen.length = 0;

  // The labels control is the same.
  const labels = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Labels of #1"]',
  )!;
  await act(async () => {
    labels.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    labels.click();
  });
  expect(onOpen).not.toHaveBeenCalled();
  expect(seen).toEqual([]);
});

it("keeps the priority target faint until hover or focus, and only for an unprioritised card", async () => {
  await render({ extras: EXTRAS });
  const target = container.querySelector<HTMLElement>(
    "[data-card-priority-target]",
  )!;
  expect(target.className).toContain("opacity-0");
  expect(target.className).toContain("group-hover/card:opacity-100");
  expect(target.className).toContain("focus-within:opacity-100");
  expect(container.querySelector("[data-card-priority]")).toBeNull();

  await render({ extras: { ...EXTRAS, priority: 1 } });
  expect(container.querySelector("[data-card-priority-target]")).toBeNull();
  expect(container.querySelector("[data-card-priority]")?.textContent).toBe(
    "Low",
  );
});

it("clears and sets a due date from the face", async () => {
  const onSetDue = vi.fn();
  await render({
    extras: { ...EXTRAS, dueAt: Date.now() + 86_400_000 },
    onSetDue,
  });
  await act(async () =>
    container
      .querySelector<HTMLButtonElement>('button[aria-label="Due date of #1"]')!
      .click(),
  );
  const clear = [
    ...document.querySelectorAll<HTMLElement>('[role^="menuitem"]'),
  ].find((el) => el.textContent === "Clear")!;
  await act(async () => clear.click());
  expect(onSetDue).toHaveBeenCalledWith(null);
});
