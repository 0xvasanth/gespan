// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

function stubStorage(entries: [string, string][]) {
  const stored = new Map(entries);
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
  });
  return stored;
}

it("turns each project's Pilot into one of its own that works on that project", async () => {
  const stored = stubStorage([
    ["rebase:project-agents", JSON.stringify({ "/code/app": "chat-1" })],
    [
      "rebase:project-agent-profiles",
      JSON.stringify({
        "/code/app": { name: "Skull", claim: "claimed" },
        "/code/site": { claim: "claimed" },
        "/code/off": { claim: "declined" },
      }),
    ],
    ["rebase:tab-group:mascots", JSON.stringify({ "/code/app": "skull" })],
  ]);
  const { isPilotSession, listPilots, pilotForSession } = await import("./pilot");

  const pilots = listPilots();
  expect(pilots.map((pilot) => pilot.projects)).toEqual([
    ["/code/app"],
    ["/code/site"],
  ]);
  expect(pilotForSession("chat-1")).toMatchObject({
    name: "Skull",
    mascot: "skull",
    projects: ["/code/app"],
    legacyProject: "/code/app",
  });
  expect(isPilotSession("chat-1")).toBe(true);
  expect(pilots[1].sessionId).toBeUndefined();
  expect(
    [...stored.keys()].filter((key) => /agent|pilots|profiles/.test(key)),
  ).toEqual([]);
});

it("migrates only once, so a deleted Pilot stays deleted", async () => {
  stubStorage([["rebase:pilots", JSON.stringify({ "/code/app": "chat-1" })]]);
  const pilot = await import("./pilot");
  const [only] = pilot.listPilots();
  pilot.removePilot(only.id);
  vi.resetModules();
  const again = await import("./pilot");
  expect(again.listPilots()).toEqual([]);
});
