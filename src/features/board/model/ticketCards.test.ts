import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import type { TicketRef } from "../../sessions/model/session";
import { resetTicketLinkCaches } from "../../sessions/model/ticketLinks";
import type { BoardColumn } from "./board";
import { createCardFromTicket, newCardColumn } from "./ticketCards";

const column = (
  id: string,
  position: number,
  role: BoardColumn["role"] = null,
): BoardColumn => ({ id, boardId: "b1", name: id, position, role });

const TICKET: TicketRef = {
  tracker: "linear",
  key: "ENG-7",
  url: null,
  title: null,
  status: null,
  statusGroup: null,
  fetchedAt: null,
};

describe("newCardColumn", () => {
  it("prefers the To do column wherever it sits", () => {
    expect(
      newCardColumn([column("a", 0), column("t", 2, "todo"), column("d", 1)])
        ?.id,
    ).toBe("t");
  });
  it("falls back to the first column by position", () => {
    expect(newCardColumn([column("b", 1), column("a", 0)])?.id).toBe("a");
    expect(newCardColumn([])).toBeUndefined();
  });
});

describe("createCardFromTicket", () => {
  let title: string;
  let fetchFails: boolean;
  const calls = (name: string) =>
    invoke.mock.calls.filter(([cmd]) => cmd === name);

  beforeEach(() => {
    resetTicketLinkCaches();
    invoke.mockReset();
    title = "ENG-7";
    fetchFails = false;
    invoke.mockImplementation(async (cmd: string, args: any) => {
      if (cmd === "board_card_create")
        return { id: "c1", boardId: "b1", title: args.title, body: "" };
      if (cmd === "ticket_fetch") {
        if (fetchFails) throw "offline";
        return { ...args.ticket, title: "Fix login", status: "Todo" };
      }
      if (cmd === "board_get")
        return {
          board: { id: "b1" },
          columns: [],
          cards: [{ id: "c1", title, body: "notes" }],
        };
      return undefined;
    });
  });

  it("creates the card titled with the key, links it, and retitles it once fetched", async () => {
    await createCardFromTicket(column("t", 0, "todo"), TICKET);
    expect(calls("board_card_create")[0][1]).toMatchObject({
      columnId: "t",
      title: "ENG-7",
    });
    const links = calls("board_card_set_ticket");
    expect(links[0][1]).toEqual({ cardId: "c1", ticket: TICKET });
    expect(links[1][1]).toMatchObject({
      cardId: "c1",
      ticket: { key: "ENG-7", title: "Fix login" },
    });
    expect(calls("board_card_update")).toEqual([
      [
        "board_card_update",
        { cardId: "c1", title: "Fix login", body: "notes" },
      ],
    ]);
  });

  it("leaves a title the user changed meanwhile", async () => {
    title = "My wording";
    await createCardFromTicket(column("t", 0, "todo"), TICKET);
    expect(calls("board_card_set_ticket")).toHaveLength(2);
    expect(calls("board_card_update")).toHaveLength(0);
  });

  it("keeps the key as the title when the fetch fails", async () => {
    fetchFails = true;
    await expect(
      createCardFromTicket(column("t", 0, "todo"), TICKET),
    ).resolves.toBe("c1");
    expect(calls("board_card_set_ticket")).toHaveLength(1);
    expect(calls("board_card_update")).toHaveLength(0);
  });
});
