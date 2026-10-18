// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { attachProjectSwipe } from "./useProjectSwipe";

let root: HTMLDivElement;
let detach: () => void;
let cwd: string;
let select: ReturnType<typeof vi.fn>;
const projects = ["/work/alpha", "remote://host/work/beta", "/work/gamma"];

function wheel(
  deltaX: number,
  deltaY = 0,
  init: WheelEventInit = {},
  target: Element = root,
) {
  const event = new WheelEvent("wheel", {
    deltaX,
    deltaY,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  // happy-dom's WheelEvent extends UIEvent and omits mouse modifiers.
  for (const key of ["ctrlKey", "metaKey", "altKey", "shiftKey"] as const) {
    Object.defineProperty(event, key, { value: init[key] ?? false });
  }
  target.dispatchEvent(event);
  return event;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(performance, "now").mockImplementation(() => Date.now());
  root = document.createElement("div");
  document.body.append(root);
  cwd = projects[1];
  select = vi.fn((path: string) => {
    cwd = path;
  });
  detach = attachProjectSwipe(root, () => ({
    cwd,
    projects,
    onSelectProject: select,
  }));
});
afterEach(() => {
  detach();
  root.remove();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("accumulates a swipe and selects once through a long momentum tail", () => {
  wheel(25);
  expect(select).not.toHaveBeenCalled();
  expect(wheel(40).defaultPrevented).toBe(true);
  expect(select).toHaveBeenCalledWith(projects[2]);
  for (let i = 0; i < 30; i++) {
    vi.advanceTimersByTime(30);
    wheel(i % 2 ? -5 : 20);
  }
  expect(select).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(250);
  wheel(-70);
  expect(select).toHaveBeenLastCalledWith(projects[1]);
  expect(select).toHaveBeenCalledTimes(2);
});

it("does not wrap at either boundary or navigate from a non-project view", () => {
  for (const [path, delta] of [
    [projects[0], -80],
    [projects[2], 80],
    ["~", 80],
  ] as const) {
    cwd = path;
    vi.advanceTimersByTime(250);
    expect(wheel(delta).defaultPrevented).toBe(true);
  }
  expect(select).not.toHaveBeenCalled();
});

it("keeps a vertical or diagonal gesture scrolling even when its tail turns horizontal", () => {
  expect(wheel(3, 25).defaultPrevented).toBe(false);
  expect(wheel(90, 2).defaultPrevented).toBe(false);
  vi.advanceTimersByTime(250);
  expect(wheel(70, 60).defaultPrevented).toBe(false);
  expect(select).not.toHaveBeenCalled();
});

it.each(["ctrlKey", "metaKey", "altKey", "shiftKey"])(
  "preserves modified wheel gestures (%s)",
  (modifier) => {
    expect(wheel(80, 0, { [modifier]: true }).defaultPrevented).toBe(false);
    wheel(80);
    expect(select).not.toHaveBeenCalled();
  },
);

it.each(["input", "textarea", "select", "div"])(
  "preserves editing in %s",
  (tag) => {
    const target = document.createElement(tag);
    if (tag === "div") target.setAttribute("contenteditable", "true");
    root.append(target);
    expect(wheel(80, 0, {}, target).defaultPrevented).toBe(false);
    expect(select).not.toHaveBeenCalled();
  },
);

it("reserves a nested horizontal scroller, including at its edge and over SVGs", () => {
  const scroller = document.createElement("div");
  scroller.style.overflowX = "auto";
  Object.defineProperties(scroller, {
    clientWidth: { value: 100 },
    scrollWidth: { value: 200 },
    scrollLeft: { value: 100 },
  });
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  scroller.append(svg);
  root.append(scroller);
  expect(wheel(80, 0, {}, svg).defaultPrevented).toBe(false);
  expect(select).not.toHaveBeenCalled();
});

it("normalizes line deltas and matches project paths with trailing slashes", () => {
  cwd = projects[0] + "/";
  wheel(4, 0, { deltaMode: 1 });
  expect(select).toHaveBeenCalledWith(projects[1]);
});

it("detaches the listener", () => {
  detach();
  expect(wheel(80).defaultPrevented).toBe(false);
  expect(select).not.toHaveBeenCalled();
});
