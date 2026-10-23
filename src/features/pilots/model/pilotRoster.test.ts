// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  addPilotProject,
  createPilot,
  findPilot,
  listPilots,
  pilotLook,
  pilotProjectsPhrase,
  pilotWorksOn,
  removePilot,
  removePilotProject,
  reorderPilots,
} from "./pilot";
import { pilotBackgroundKey, pilotChatBackground } from "./pilotBackground";
import { saveProjectChatBackgroundSettings } from "../../projects/model/projectChatBackground";

beforeEach(() => {
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
  });
});
afterEach(() => vi.unstubAllGlobals());

it("gives each new Pilot a mascot and color the others are not using", () => {
  const first = createPilot();
  const second = createPilot();
  expect(second.mascot).not.toBe(first.mascot);
  expect(second.color).not.toBe(first.color);
  expect(pilotLook(first).name).toMatch(/^Pilot[A-Z]/);
  expect(first.projects).toEqual([]);
});

it("adds each project once and takes one away by its path", () => {
  const { id } = createPilot(["/code/app"]);
  addPilotProject(id, "/code/site");
  addPilotProject(id, "/code/app/");
  expect(findPilot(id)?.projects).toEqual(["/code/app", "/code/site"]);
  expect(pilotWorksOn(findPilot(id)!, "/code/site/")).toBe(true);
  removePilotProject(id, "/code/app/");
  expect(findPilot(id)?.projects).toEqual(["/code/site"]);
});

it("keeps the rail's order and forgets a removed Pilot", () => {
  const a = createPilot();
  const b = createPilot();
  const c = createPilot();
  reorderPilots([c.id, a.id, b.id]);
  expect(listPilots().map((pilot) => pilot.id)).toEqual([c.id, a.id, b.id]);
  removePilot(a.id);
  expect(listPilots().map((pilot) => pilot.id)).toEqual([c.id, b.id]);
});

it("names its projects the way a sentence would", () => {
  const project = (name: string) => ({ path: `/code/${name}`, name });
  expect(pilotProjectsPhrase([])).toBe("");
  expect(pilotProjectsPhrase([project("app")])).toBe("app");
  expect(
    pilotProjectsPhrase([project("app"), project("site"), project("api")]),
  ).toBe("app, site and api");
});

it("forgets its background along with the Pilot", () => {
  const pilot = createPilot();
  saveProjectChatBackgroundSettings(pilotBackgroundKey(pilot.id), {
    path: "/bg/pilot.png",
    emptyOpacity: 0.24,
    sessionOpacity: 0.24,
    scope: "all",
    effect: "gradient-blur",
  });
  expect(pilotChatBackground(pilot.id)?.path).toBe("/bg/pilot.png");
  removePilot(pilot.id);
  expect(pilotChatBackground(pilot.id)).toBeNull();
});
