// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { resetTicketLinkCaches } from "../../sessions/model/ticketLinks";
import type { TicketRef } from "../../sessions/model/session";
import type { BoardCard } from "../model/board";
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

async function render() {
  const onOpen = vi.fn();
  await act(async () =>
    root.render(
      createElement(BoardCardItem, {
        card: CARD,
        done: false,
        link: null,
        ticket: TICKET,
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
