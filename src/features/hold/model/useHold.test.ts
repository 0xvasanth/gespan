// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useHold, type HoldActions } from "./useHold";

let container: HTMLDivElement;
let root: Root | null = null;
let latest: ReturnType<typeof useHold> | null = null;

function Probe({ actions }: { actions: HoldActions }) {
  latest = useHold(actions);
  return null;
}

const actions = (busy: boolean): HoldActions => ({
  sessions: () => [
    { id: "a", busy, pilotChat: false, awaitingInput: false, queued: 0 },
  ],
  holdTurn: vi.fn(),
  resume: vi.fn(() => false),
});

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.append(container);
  localStorage.clear();
});

afterEach(() => {
  if (root) {
    act(() => root!.unmount());
    root = null;
  }
  container.remove();
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("useHold", () => {
  it("asks for confirmation, then holds and re-renders with the new phase", async () => {
    root = createRoot(container);
    act(() => root!.render(createElement(Probe, { actions: actions(false) })));
    expect(latest!.phase).toBeNull();
    let toggled!: Promise<void>;
    act(() => {
      toggled = latest!.hold.toggle();
    });
    expect(latest!.confirming).toBe(true);
    await act(async () => {
      latest!.answer(true);
      await toggled;
    });
    expect(latest!.confirming).toBe(false);
    expect(latest!.phase).toBe("onHold");
  });

  it("cancelling the confirmation leaves everything running", async () => {
    root = createRoot(container);
    const busy = actions(true);
    act(() => root!.render(createElement(Probe, { actions: busy })));
    let toggled!: Promise<void>;
    act(() => {
      toggled = latest!.hold.toggle();
    });
    await act(async () => {
      latest!.answer(false);
      await toggled;
    });
    expect(latest!.phase).toBeNull();
    expect(busy.holdTurn).not.toHaveBeenCalled();
  });
});
