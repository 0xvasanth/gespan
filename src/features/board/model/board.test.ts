import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { BOARD_CHANGED, linkCard, subscribeBoard } from "./board";

describe("subscribeBoard", () => {
  let handler: (event: { payload: { boardId: string } }) => void;
  const stop = vi.fn();

  beforeEach(() => {
    stop.mockReset();
    listen.mockReset();
    listen.mockImplementation(async (_name: string, cb: typeof handler) => {
      handler = cb;
      return stop;
    });
  });

  it("only fires for changes to its own board", async () => {
    const onChange = vi.fn();
    subscribeBoard("b1", onChange);
    await Promise.resolve();

    expect(listen).toHaveBeenCalledWith(BOARD_CHANGED, expect.any(Function));
    handler({ payload: { boardId: "b2" } });
    expect(onChange).not.toHaveBeenCalled();
    handler({ payload: { boardId: "b1" } });
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("stops listening on unsubscribe", async () => {
    const unsubscribe = subscribeBoard("b1", vi.fn());
    await Promise.resolve();
    await Promise.resolve();
    unsubscribe();
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it("stops listening when unsubscribed before listen resolves", async () => {
    const unsubscribe = subscribeBoard("b1", vi.fn());
    unsubscribe();
    await Promise.resolve();
    await Promise.resolve();
    expect(stop).toHaveBeenCalledTimes(1);
  });
});

describe("invoke wrappers", () => {
  beforeEach(() => invoke.mockReset());

  it("sends null link fields to clear a card link", async () => {
    await linkCard("c1", null);
    expect(invoke).toHaveBeenCalledWith("board_card_link", {
      cardId: "c1",
      linkKind: null,
      linkId: null,
    });
  });

  it("sends the link kind and id to link a card", async () => {
    await linkCard("c1", { kind: "pilot", id: "p1" });
    expect(invoke).toHaveBeenCalledWith("board_card_link", {
      cardId: "c1",
      linkKind: "pilot",
      linkId: "p1",
    });
  });
});
