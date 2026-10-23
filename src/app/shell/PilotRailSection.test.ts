// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createPilot } from "../../features/pilots/model/pilot";
import { loadPilotsEnabled } from "../../features/settings/model/settings";
import { PilotRailSection } from "./PilotRailSection";

let root: Root;
let container: HTMLElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function render(props: Partial<Parameters<typeof PilotRailSection>[0]> = {}) {
  const onCreate = vi.fn();
  act(() =>
    root.render(
      createElement(PilotRailSection, {
        states: new Map(),
        onOpen: () => {},
        onCreate,
        onDelete: () => {},
        ...props,
      }),
    ),
  );
  return onCreate;
}

const intro = () => document.body.querySelector('[aria-label="Meet Pilots"]');
const button = (label: string) =>
  [...document.body.querySelectorAll("button")].find(
    (entry) => entry.textContent === label,
  )!;

it("shows the intro card in place of the list while there is no pilot", () => {
  const onCreate = render();
  expect(intro()).not.toBeNull();
  expect(container.querySelector('[aria-label="New pilot"]')).toBeNull();

  act(() => button("Create pilot").click());
  expect(onCreate).toHaveBeenCalledOnce();
});

it("turns pilots off from the intro's switch", () => {
  render();
  act(() => container.querySelector<HTMLElement>('[role="switch"]')!.click());
  expect(loadPilotsEnabled()).toBe(false);
});

it("lists pilots once there are some, with the plus in the header", () => {
  createPilot();
  render();
  expect(intro()).toBeNull();
  expect(container.querySelectorAll('[aria-label="New pilot"]')).toHaveLength(1);
});

it("counts a pilot's unread replies, except the one open", () => {
  const pilot = createPilot();
  const badge = () => container.querySelector("[data-pilot-unread]");
  render({ unseenCounts: new Map([[pilot.id, 3]]) });
  expect(badge()?.textContent).toBe("3");
  expect(
    container.querySelector(`[aria-label*="3 new replies"]`),
  ).not.toBeNull();

  render({ unseenCounts: new Map([[pilot.id, 120]]) });
  expect(badge()?.textContent).toBe("99+");

  render({ activeId: pilot.id, unseenCounts: new Map([[pilot.id, 3]]) });
  expect(badge()).toBeNull();
});
