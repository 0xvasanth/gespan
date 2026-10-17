// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useReleasedIds } from "./useReleasedIds";

it("releases only shown ids that fell out of the most recent", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const seen: ReadonlySet<string>[] = [];
  function Probe({ current }: { current: string | null }) {
    seen.push(useReleasedIds(current, 2));
    return null;
  }
  const root = createRoot(document.createElement("div"));
  const show = (current: string | null) =>
    act(() => root.render(createElement(Probe, { current })));

  show("a");
  show("b");
  // Nothing shown has dropped out yet, and unseen ids are never released.
  expect([...seen.at(-1)!]).toEqual([]);
  show("c");
  expect([...seen.at(-1)!]).toEqual(["a"]);
  show(null);
  show("d");
  expect([...seen.at(-1)!].sort()).toEqual(["a", "b"]);
  // Showing a released id reclaims it.
  show("a");
  expect([...seen.at(-1)!].sort()).toEqual(["b", "c"]);
  // Re-rendering the same current id keeps the same set.
  const before = seen.at(-1);
  show("a");
  expect(seen.at(-1)).toBe(before);
  act(() => root.unmount());
});
