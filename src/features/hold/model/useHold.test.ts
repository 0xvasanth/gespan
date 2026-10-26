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
  it("re-renders with the new phase when the hold changes", () => {
    root = createRoot(container);
    act(() => root!.render(createElement(Probe, { actions: actions(false) })));
    expect(latest!.phase).toBeNull();
    act(() => latest!.hold.toggle());
    expect(latest!.phase).toBe("onHold");
    act(() => latest!.hold.toggle());
    expect(latest!.phase).toBeNull();
  });
});
