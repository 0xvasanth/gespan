// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useRecentIds } from "./useRecentIds";

it("keeps the last distinct current ids, most recent first", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const seen: (readonly string[])[] = [];
  function Probe({ current }: { current: string | null }) {
    seen.push(useRecentIds(current, 3));
    return null;
  }
  const root = createRoot(document.createElement("div"));
  const show = (current: string | null) =>
    act(() => root.render(createElement(Probe, { current })));

  for (const id of ["a", "b", "c", "b", null, "d"]) show(id);
  expect(seen.at(-1)).toEqual(["d", "b", "c"]);
  // Re-rendering the same current id keeps the same array.
  const before = seen.at(-1);
  show("d");
  expect(seen.at(-1)).toBe(before);
  act(() => root.unmount());
});
