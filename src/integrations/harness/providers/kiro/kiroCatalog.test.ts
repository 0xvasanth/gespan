import { beforeEach, describe, expect, it, vi } from "vitest";

let line: ((value: string) => void) | undefined;
const sent: Record<string, unknown>[] = [];
const killed: string[] = [];
/** What the fake `session/new` returns; `undefined` makes it fail. */
let sessionNew: unknown;

vi.mock("../../../../platform/tauri/fs", () => ({
  homeDir: async () => "/home/kiro-user",
}));

vi.mock("../../core/child", () => ({
  resolveKiroBinary: async () => ({ path: "/fake/kiro-cli" }),
  spawnChild: async () => undefined,
  killChild: async (id: string) => {
    killed.push(id);
  },
  unwatchChild: () => undefined,
  watchChild: (_id: string, onLine: (value: string) => void) => {
    line = onLine;
  },
  writeChild: async (_id: string, raw: string) => {
    const message = JSON.parse(raw) as Record<string, unknown>;
    sent.push(message);
    queueMicrotask(() => {
      const reply = (result?: unknown, error?: unknown) =>
        line!(
          JSON.stringify({
            jsonrpc: "2.0",
            id: message.id,
            ...(error ? { error } : { result }),
          }),
        );
      if (message.method === "initialize") reply({});
      if (message.method === "session/new") {
        if (sessionNew === undefined) reply(undefined, { message: "no session" });
        else reply(sessionNew);
      }
    });
  },
}));

const { discoverKiroModels, refreshKiroCatalog } = await import("./kiroCatalog");
const { modelsFor, hasLiveCatalog, resetHarnessModelOverlays } = await import(
  "../../../../features/sessions/model/models"
);

const withModels = {
  sessionId: "probe",
  configOptions: [
    {
      id: "model",
      category: "model",
      currentValue: "b",
      options: [
        { value: "a", name: "Model A" },
        { value: "b", name: "Model B" },
      ],
    },
  ],
};

beforeEach(() => {
  sent.length = 0;
  killed.length = 0;
  sessionNew = withModels;
  resetHarnessModelOverlays();
});

describe("discoverKiroModels", () => {
  it("probes a throwaway session in the home directory and returns its models", async () => {
    const models = await discoverKiroModels();
    expect(models.map((model) => model.nativeId)).toEqual(["b", "a"]);
    const created = sent.find((message) => message.method === "session/new");
    expect(created?.params).toEqual({ cwd: "/home/kiro-user", mcpServers: [] });
  });

  it("always tears the probe process down", async () => {
    await discoverKiroModels();
    expect(killed).toHaveLength(1);
    sessionNew = undefined;
    await expect(discoverKiroModels()).rejects.toThrow(/no session/);
    expect(killed).toHaveLength(2);
  });
});

describe("refreshKiroCatalog", () => {
  it("publishes discovered models to the picker", async () => {
    expect(hasLiveCatalog("kiro")).toBe(false);
    await refreshKiroCatalog();
    expect(hasLiveCatalog("kiro")).toBe(true);
    expect(modelsFor("kiro").map((model) => model.id)).toEqual([
      "kiro:b",
      "kiro:a",
    ]);
  });

  it("leaves the default entry alone when Kiro advertises no models", async () => {
    sessionNew = { sessionId: "probe" };
    await refreshKiroCatalog();
    expect(hasLiveCatalog("kiro")).toBe(false);
    expect(modelsFor("kiro").map((model) => model.id)).toEqual(["kiro:default"]);
  });

  it("swallows a failed probe so the picker still works", async () => {
    sessionNew = undefined;
    await expect(refreshKiroCatalog()).resolves.toBeUndefined();
    expect(hasLiveCatalog("kiro")).toBe(false);
  });
});
