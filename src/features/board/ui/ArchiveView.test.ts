// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { BoardCard } from "../model/board";
import { ArchiveView } from "./ArchiveView";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const events = vi.hoisted(() => ({
  listeners: new Set<(event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (_name: string, handler: (event: { payload: unknown }) => void) => {
      events.listeners.add(handler);
      return Promise.resolve(() => events.listeners.delete(handler));
    },
  ),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));

function card(id: string, number: number, title: string): BoardCard {
  return {
    id,
    boardId: "b1",
    columnId: "backlog",
    number,
    title,
    body: "",
    position: 0,
    linkKind: null,
    linkId: null,
    createdAt: 1,
    updatedAt: 1,
  };
}

let archived: BoardCard[];
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  events.listeners.clear();
  // Newest first, as the store returns them.
  archived = [card("a", 1, "Alpha"), card("b", 2, "Beta")];
  vi.mocked(invoke).mockReset();
  vi.mocked(ask).mockReset();
  vi.mocked(invoke).mockImplementation(async (cmd, args) => {
    const id = (args as { cardId?: string } | undefined)?.cardId;
    if (cmd === "board_archived_cards") return archived.map((c) => ({ ...c }));
    if (cmd === "board_card_unarchive" || cmd === "board_card_delete") {
      archived = archived.filter((c) => c.id !== id);
      return undefined;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
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

async function flush() {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

async function render(overrides: Partial<ComponentProps<typeof ArchiveView>>) {
  const props: ComponentProps<typeof ArchiveView> = {
    boardId: "b1",
    columnNames: new Map([["backlog", "Backlog"]]),
    pendingDelete: null,
    onClose: vi.fn(),
    onError: vi.fn(),
    ...overrides,
  };
  await act(async () => root.render(createElement(ArchiveView, props)));
  await flush();
  return props;
}

const rows = () =>
  [...container.querySelectorAll("[data-archived-card]")].map((el) =>
    el.getAttribute("data-archived-card"),
  );
const button = (id: string, text: string) =>
  [
    ...container.querySelectorAll<HTMLButtonElement>(
      `[data-archived-card='${id}'] button`,
    ),
  ].find((el) => el.textContent === text)!;
function changed() {
  for (const listener of [...events.listeners]) {
    listener({ payload: { boardId: "b1" } });
  }
}

describe("ArchiveView", () => {
  it("lists the board's archived cards newest first with number and column", async () => {
    await render({});
    expect(container.querySelector("aside")?.getAttribute("aria-label")).toBe(
      "Archive",
    );
    expect(rows()).toEqual(["a", "b"]);
    expect(
      container.querySelector("[data-archived-card='b']")!.textContent,
    ).toContain("#2 · Backlog");
  });

  it("leaves out the card whose delete the undo toast is holding", async () => {
    await render({ pendingDelete: "a" });
    expect(rows()).toEqual(["b"]);
    archived = [card("a", 1, "Alpha")];
    changed();
    await flush();
    // Only the held card is archived: the view reads as empty.
    expect(rows()).toEqual([]);
    expect(container.textContent).toContain("No archived cards");
  });

  it("reloads when the board changes and restores a card", async () => {
    await render({});
    await act(async () => button("a", "Restore").click());
    expect(invoke).toHaveBeenCalledWith("board_card_unarchive", {
      cardId: "a",
    });
    changed();
    await flush();
    expect(rows()).toEqual(["b"]);
  });

  it("Delete forever asks first and deletes only on yes", async () => {
    const props = await render({});
    vi.mocked(ask).mockResolvedValue(false);
    await act(async () => button("b", "Delete forever").click());
    await flush();
    expect(vi.mocked(ask).mock.calls[0][0]).toBe(
      "Delete card #2 “Beta” forever? Its notes and activity are removed.",
    );
    expect(vi.mocked(ask).mock.calls[0][1]).toMatchObject({
      okLabel: "Delete forever",
    });
    expect(invoke).not.toHaveBeenCalledWith("board_card_delete", {
      cardId: "b",
    });

    // A dialog that can't open counts as no.
    vi.mocked(ask).mockRejectedValue(new Error("no dialog"));
    await act(async () => button("b", "Delete forever").click());
    await flush();
    expect(invoke).not.toHaveBeenCalledWith("board_card_delete", {
      cardId: "b",
    });

    vi.mocked(ask).mockResolvedValue(true);
    await act(async () => button("b", "Delete forever").click());
    await flush();
    expect(invoke).toHaveBeenCalledWith("board_card_delete", { cardId: "b" });
    expect(props.onError).not.toHaveBeenCalled();
  });

  it("reports a failed load instead of claiming the archive is empty", async () => {
    vi.mocked(invoke).mockRejectedValue("database is locked");
    const props = await render({});
    expect(props.onError).toHaveBeenCalledWith(
      "Couldn't load the archive: database is locked",
    );
    expect(container.textContent).not.toContain("No archived cards");
  });

  it("closes on Escape", async () => {
    const props = await render({});
    act(() => {
      container
        .querySelector("aside")!
        .dispatchEvent(
          new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
        );
    });
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });
});
