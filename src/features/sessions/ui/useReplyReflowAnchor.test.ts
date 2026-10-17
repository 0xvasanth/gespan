// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useReplyReflowAnchor } from "./useReplyReflowAnchor";

it("keeps the reader anchored through paragraph gaps and a long wheel gesture", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.push(callback);
    return frames.length;
  });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  let resize!: ResizeObserverCallback;
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: ResizeObserverCallback) {
        resize = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
  const host = document.createElement("div");
  const scroller = document.createElement("div");
  scroller.innerHTML =
    '<div><div data-chat-message-role="assistant">A visible reply</div></div>';
  document.body.append(host, scroller);
  const text = scroller.querySelector("[data-chat-message-role]")!.firstChild!;
  let width = 200;
  let glyphTop = 48;
  let now = 0;
  vi.spyOn(performance, "now").mockImplementation(() => now);
  vi.spyOn(Range.prototype, "getBoundingClientRect").mockImplementation(
    () => ({ top: glyphTop - scroller.scrollTop }) as DOMRect,
  );
  Object.defineProperty(scroller, "clientWidth", { get: () => width });
  scroller.getBoundingClientRect = () =>
    ({ top: 0, left: 0, width, height: 200 }) as DOMRect;
  const descriptor = Object.getOwnPropertyDescriptor(
    document,
    "caretRangeFromPoint",
  );
  Object.defineProperty(document, "caretRangeFromPoint", {
    configurable: true,
    value: (_x: number, y: number) => {
      const range = document.createRange();
      if (y === 32) range.selectNodeContents(scroller);
      else {
        range.setStart(text, 3);
        range.setEnd(text, 4);
      }
      return range;
    },
  });
  const remember = vi.fn();
  const following = { current: false };
  function Probe() {
    useReplyReflowAnchor(scroller, true, following, remember);
    return null;
  }
  const root = createRoot(host);
  try {
    act(() => root.render(createElement(Probe)));
    act(() => frames.shift()!(0));
    width = 100;
    glyphTop = 148;
    act(() => resize([], {} as ResizeObserver));
    expect(scroller.scrollTop).toBe(100);
    expect(remember).toHaveBeenCalledWith(scroller);

    // Momentum continues after the original wheel event's quiet period.
    // Each scroll must update the reader's glyph until the gesture settles.
    scroller.dispatchEvent(new WheelEvent("wheel", { deltaY: 40 }));
    for (const [time, movement] of [[0, 20], [150, 30], [250, 40]]) {
      now = time;
      scroller.scrollTop = 100 + movement;
      scroller.dispatchEvent(new Event("scroll"));
      act(() => {
        while (frames.length) frames.shift()!(now);
      });
    }
    width = 200;
    glyphTop = 248;
    act(() => resize([], {} as ResizeObserver));
    expect(scroller.scrollTop).toBe(240);

    // A native scroll can reach the main thread after the input's quiet
    // window, even when the wheel's animation frame has already run.
    now = 1000;
    scroller.dispatchEvent(new WheelEvent("wheel", { deltaY: 40 }));
    act(() => {
      while (frames.length) frames.shift()!(now);
    });
    now = 1250;
    scroller.scrollTop = 280;
    scroller.dispatchEvent(new Event("scroll"));
    act(() => {
      while (frames.length) frames.shift()!(now);
    });
    width = 100;
    glyphTop = 348;
    act(() => resize([], {} as ResizeObserver));
    expect(scroller.scrollTop).toBe(380);
  } finally {
    act(() => root.unmount());
    host.remove();
    scroller.remove();
    if (descriptor)
      Object.defineProperty(document, "caretRangeFromPoint", descriptor);
    else Reflect.deleteProperty(document, "caretRangeFromPoint");
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});
