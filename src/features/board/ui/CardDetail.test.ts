// @vitest-environment happy-dom
import { act, createElement, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { BoardCard } from "../model/board";
import { CardDetail } from "./CardDetail";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));

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

async function renderDetail() {
  const props: ComponentProps<typeof CardDetail> = {
    card: CARD,
    columnName: "In Progress",
    link: null,
    linkTargets: [],
    onClose: vi.fn(),
    onOpenLinkedWork: vi.fn(),
    onError: vi.fn(),
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
